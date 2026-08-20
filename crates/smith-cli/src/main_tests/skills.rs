// -- `/skills` ----------------------------------------------------------------

/// A user root and a project root, neither of which declares a skill yet.
fn skill_roots() -> (tempfile::TempDir, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    std::fs::create_dir_all(home.path().join(".smith")).expect("a user `.smith`");
    std::fs::create_dir_all(project.path().join(".smith")).expect("a project `.smith`");
    std::fs::write(
        project.path().join(".smith/config.toml"),
        LOCAL_COMMAND_CONFIG,
    )
    .expect("a project config");
    (home, project)
}

fn write_skill_body(root: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let directory = root.join("skills").join(name);
    std::fs::create_dir_all(&directory).expect("a skill directory");
    let path = directory.join("SKILL.md");
    std::fs::write(&path, body).expect("a skill body");
    path
}

fn skill_context(
    home: &std::path::Path,
    project: &std::path::Path,
) -> (
    smith_runtime::skills::ResolvedSmithSkills,
    crate::skills::SkillContext,
) {
    let (sources, context) = crate::skills::SkillContext::compose(
        smith_runtime::built_in_skills::built_in_sources(),
        home,
        project,
    )
    .expect("a skill context");
    (sources.resolve().expect("the catalog resolves"), context)
}

const SKILL_BODY: &str = "---\ndescription: Review Rust implementation boundaries\n---\n\nRead the unsafe blocks first.\n";

#[tokio::test]
async fn skills_list_groups_by_layer_and_names_the_winner() {
    let (home, project) = skill_roots();
    write_skill_body(&home.path().join(".smith"), "smith.security", SKILL_BODY);
    write_skill_body(&home.path().join(".smith"), "rust-review", SKILL_BODY);

    let (resolved, context) = skill_context(&home.path().join(".smith"), project.path());
    let listed = context.render_list(resolved.index());

    assert!(listed.contains("built-in"), "{listed}");
    assert!(listed.contains("user"), "{listed}");
    assert!(
        listed.contains("rust-review · active · Review Rust implementation boundaries"),
        "{listed}"
    );
    // The user copy wins the name; the built-in it displaced is still listed,
    // because "which one am I getting" is the question the catalog raises.
    assert!(
        listed.contains("smith.security · shadowed by the user skill of the same name"),
        "a shadowed built-in must stay visible and say so: {listed}"
    );
    assert_eq!(
        listed
            .matches("smith.security")
            .count(),
        2,
        "both entries for a shadowed name are listed: {listed}"
    );
}

#[tokio::test]
async fn skills_list_reports_discovery_problems() {
    let (home, project) = skill_roots();
    write_skill_body(&home.path().join(".smith"), "good", SKILL_BODY);
    write_skill_body(
        &home.path().join(".smith"),
        "broken",
        "no frontmatter at all\n",
    );

    let (resolved, context) = skill_context(&home.path().join(".smith"), project.path());
    let listed = context.render_list(resolved.index());

    assert!(listed.contains("good · active"), "{listed}");
    assert!(listed.contains("not loaded"), "{listed}");
    assert!(
        listed.contains("broken ·") && listed.contains("`---`"),
        "a refused file is named with its reason: {listed}"
    );
}

#[tokio::test]
async fn an_untrusted_project_skill_says_what_to_do_about_it() {
    let (home, project) = skill_roots();
    write_skill_body(&project.path().join(".smith"), "deploy", SKILL_BODY);

    let (resolved, context) = skill_context(&home.path().join(".smith"), project.path());
    let listed = context.render_list(resolved.index());

    assert!(
        listed.contains("deploy · withheld · needs approval — run `/skills trust deploy`"),
        "{listed}"
    );
}

#[tokio::test]
async fn skills_trust_shows_path_and_digest_before_recording() {
    let (home, project) = skill_roots();
    let path = write_skill_body(&project.path().join(".smith"), "deploy", SKILL_BODY);
    let (_, context) = skill_context(&home.path().join(".smith"), project.path());

    let confirmation = context.confirmation("deploy").expect("a confirmation");
    assert!(
        confirmation.contains(".smith/skills/deploy/SKILL.md"),
        "the project-relative path is shown: {confirmation}"
    );
    assert!(
        confirmation.contains("content ") && confirmation.contains("nothing has been decided"),
        "{confirmation}"
    );
    assert!(
        !confirmation.contains("Read the unsafe blocks first"),
        "the body must not argue for its own trust: {confirmation}"
    );
    // Rendering a confirmation decides nothing: the skill is still withheld.
    let (resolved, _) = skill_context(&home.path().join(".smith"), project.path());
    assert!(resolved.index().iter().all(|entry| {
        entry.layer != smith_runtime::skills::SmithSkillLayer::Workspace || !entry.activatable
    }));

    let notice = context.trust("deploy").expect("the decision records");
    assert!(notice.contains("idle boundary"), "{notice}");

    // The next composition — the one the idle boundary triggers — admits it.
    let (resolved, context) = skill_context(&home.path().join(".smith"), project.path());
    assert!(resolved.index().iter().all(|entry| entry.activatable));
    assert!(
        context.render_list(resolved.index()).contains("deploy · active"),
        "the trusted skill is active"
    );

    // And the decision covers exactly that content, not that path.
    std::fs::write(&path, "---\ndescription: Review\n---\n\nRewritten by a later commit.\n")
        .expect("a later commit");
    let (resolved, context) = skill_context(&home.path().join(".smith"), project.path());
    let listed = context.render_list(resolved.index());
    assert!(
        listed.contains("deploy · withheld · its content changed"),
        "{listed}"
    );
}

#[tokio::test]
async fn skills_trust_unknown_name_lists_known_names() {
    let (home, project) = skill_roots();
    write_skill_body(&project.path().join(".smith"), "deploy", SKILL_BODY);
    let (_, context) = skill_context(&home.path().join(".smith"), project.path());

    let error = context.confirmation("depoly").expect_err("no such skill");
    assert!(error.contains("this project declares: deploy"), "{error}");

    let (home, project) = skill_roots();
    let (_, context) = skill_context(&home.path().join(".smith"), project.path());
    let error = context.confirmation("anything").expect_err("no such skill");
    assert!(error.contains("declares no skills"), "{error}");
}

#[tokio::test]
async fn a_user_skill_is_never_withheld_and_a_malformed_one_does_not_hide_it() {
    let (home, project) = skill_roots();
    write_skill_body(&home.path().join(".smith"), "rust-review", SKILL_BODY);
    write_skill_body(&home.path().join(".smith"), "half-written", "---\n");

    let (resolved, context) = skill_context(&home.path().join(".smith"), project.path());
    let listed = context.render_list(resolved.index());
    assert!(listed.contains("rust-review · active"), "{listed}");
    assert!(listed.contains("half-written ·"), "{listed}");
    assert!(
        !listed.contains("half-written · active"),
        "a malformed skill must not be indexed: {listed}"
    );
}
