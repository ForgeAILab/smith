//! Workspace dependency-direction checks that run on every supported host.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{Attribute, Item, Meta, Token, UseTree, Visibility};
use toml::{Table, Value};

#[test]
fn the_full_runtime_facade_is_a_smith_runtime_production_dependency_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root");
    let workspace = manifest(&root.join("Cargo.toml"));
    let members = workspace
        .get("workspace")
        .and_then(Value::as_table)
        .and_then(|workspace| workspace.get("members"))
        .and_then(Value::as_array)
        .expect("workspace.members");

    let mut owners = BTreeSet::new();
    for member in members {
        let member = member.as_str().expect("a string workspace member");
        let package = manifest(&root.join(member).join("Cargo.toml"));
        if has_production_facade_dependency(&package) {
            owners.insert(member.to_owned());
        }
    }

    assert_eq!(
        owners,
        BTreeSet::from(["crates/smith-runtime".to_owned()]),
        "the full facade must enter production composition only through smith-runtime"
    );
}

fn manifest(path: &Path) -> Table {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
    toml::from_str(&source).unwrap_or_else(|error| panic!("parsing {}: {error}", path.display()))
}

fn has_production_facade_dependency(manifest: &Table) -> bool {
    ["dependencies", "build-dependencies"]
        .into_iter()
        .any(|section| dependency_table_has_facade(manifest.get(section)))
        || manifest
            .get("target")
            .and_then(Value::as_table)
            .is_some_and(|targets| {
                targets.values().any(|target| {
                    target.as_table().is_some_and(|target| {
                        ["dependencies", "build-dependencies"]
                            .into_iter()
                            .any(|section| dependency_table_has_facade(target.get(section)))
                    })
                })
            })
}

fn dependency_table_has_facade(dependencies: Option<&Value>) -> bool {
    dependencies
        .and_then(Value::as_table)
        .is_some_and(|dependencies| {
            dependencies
                .iter()
                .any(|(alias, dependency)| dependency_package(alias, dependency) == "agent-runtime")
        })
}

fn dependency_package<'a>(alias: &'a str, dependency: &'a Value) -> &'a str {
    dependency
        .as_table()
        .and_then(|dependency| dependency.get("package"))
        .and_then(Value::as_str)
        .unwrap_or(alias)
}

#[test]
fn presentation_crates_use_the_smith_client_event_protocol() {
    let root = workspace_root();
    for relative in ["crates/smith-cli", "crates/smith-tui"] {
        visit_crate_rust(&root.join(relative), &mut |path, source| {
            for import in imports(source, false) {
                assert!(
                    !path_starts_with(&import, &["agent_runtime_core", "event"]),
                    "presentation source {} imports the canonical event vocabulary: {}",
                    path.display(),
                    import.join("::"),
                );
                assert!(
                    !imports_runtime_session_handle(&import),
                    "presentation source {} imports the canonical session handle: {}",
                    path.display(),
                    import.join("::"),
                );
            }
        });
    }
}

#[test]
fn smith_tui_has_no_smith_config_dependency() {
    assert_no_dependencies("smith-tui", &["smith-config"]);
}

#[test]
fn smith_client_has_no_terminal_library_dependency() {
    assert_no_dependencies("smith-client", &["ratatui", "crossterm"]);
}

#[test]
fn headless_code_does_not_import_smith_tui() {
    visit_rust(
        &workspace_root().join("crates/smith-cli/src/headless"),
        &mut |path, source| {
            for import in imports(source, false) {
                assert!(
                    !path_starts_with(&import, &["smith_tui"]),
                    "headless source {} imports smith_tui: {}",
                    path.display(),
                    import.join("::"),
                );
            }
        },
    );
}

#[test]
fn production_smith_cli_has_no_glob_imports() {
    let directory = workspace_root().join("crates/smith-cli/src");
    visit_rust(&directory, &mut |path, source| {
        if path
            .strip_prefix(&directory)
            .expect("CLI source")
            .components()
            .any(|component| matches!(component.as_os_str().to_str(), Some("tests" | "main_tests")))
        {
            return;
        }
        for import in imports(source, true) {
            assert!(
                import.last().is_none_or(|segment| segment != "*"),
                "production CLI source {} has a glob import: {}",
                path.display(),
                import.join("::"),
            );
        }
    });
}

#[test]
fn smith_cli_event_streams_are_created_only_by_the_screen_runner() {
    let directory = workspace_root().join("crates/smith-cli/src");
    // Standalone screens own their reader; the interactive process borrows a
    // runner-created reader across hosts and embedded screens. A constructor
    // in the TUI loop would reintroduce a reader handoff on every rebuild.
    let allowed = BTreeSet::from([PathBuf::from("screen_runner.rs")]);
    let mut owners = BTreeSet::new();
    visit_rust(&directory, &mut |path, source| {
        let count = source.matches("EventStream::new").count();
        if count == 0 {
            return;
        }
        let relative = path.strip_prefix(&directory).expect("CLI source");
        assert!(
            allowed.contains(relative),
            "{} creates an event stream outside the shared input owner",
            path.display(),
        );
        assert_eq!(
            count,
            1,
            "{} must own exactly one event stream",
            path.display(),
        );
        owners.insert(relative.to_path_buf());
    });
    assert_eq!(
        owners, allowed,
        "the runner must provide the single event-stream constructor"
    );
}

// Outside consumers were audited across other crates, integration tests, and
// examples. Modules without a named-path consumer are crate-private; profile
// evidence and renewable credential types remain reachable through public APIs.
const PUBLIC_RUNTIME_MODULES: &[&str] = &[
    "advisor",
    "artifact",
    "background_tasks",
    "built_in_skills",
    "cache_controller",
    "cache_lifecycle",
    "chatgpt",
    "checkpoint",
    "client",
    "delegation",
    "factory",
    "harness",
    "host",
    "journal",
    "mcp",
    "memory",
    "model_catalog",
    "pool",
    "pool_state",
    "probe",
    "project_instructions",
    "prompt",
    "reasoning",
    "resume_capsule",
    "rotation",
    "session",
    "skills",
    "summary",
    "tool_output",
    "transport",
    "xai",
];

#[test]
fn smith_runtime_public_modules_match_the_allow_list() {
    let path = workspace_root().join("crates/smith-runtime/src/lib.rs");
    let source = fs::read_to_string(&path).expect("runtime crate root");
    let file = syn::parse_file(&source).expect("runtime crate root syntax");
    let actual: BTreeSet<_> = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Mod(module) if matches!(module.vis, Visibility::Public(_)) => {
                Some(module.ident.to_string())
            }
            _ => None,
        })
        .collect();
    let expected: BTreeSet<_> = PUBLIC_RUNTIME_MODULES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(
        actual,
        expected,
        "public runtime modules changed; unexpected: {:?}; missing: {:?}",
        actual.difference(&expected).collect::<Vec<_>>(),
        expected.difference(&actual).collect::<Vec<_>>(),
    );
}

/// Longest a `.rs` file under `crates/` may grow before it is split into child
/// modules under the same module root.
const MAX_RUST_FILE_LINES: usize = 1_500;

#[test]
fn every_rust_file_is_within_the_line_budget() {
    let root = workspace_root();
    let mut over = Vec::new();
    visit_rust(&root.join("crates"), &mut |path, source| {
        let lines = source.lines().count();
        if lines > MAX_RUST_FILE_LINES {
            let path = path.strip_prefix(&root).unwrap_or(path);
            over.push(format!("{} ({lines} lines)", path.display()));
        }
    });
    over.sort();
    assert!(
        over.is_empty(),
        "split these into child modules of at most {MAX_RUST_FILE_LINES} lines:\n{}",
        over.join("\n"),
    );
}

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root")
}

fn assert_no_dependencies(crate_name: &str, forbidden: &[&str]) {
    let root = workspace_root();
    let workspace = manifest(&root.join("Cargo.toml"));
    let package = manifest(&root.join("crates").join(crate_name).join("Cargo.toml"));
    let check = |table: &Table| {
        for section in ["dependencies", "build-dependencies", "dev-dependencies"] {
            if let Some(dependencies) = table.get(section).and_then(Value::as_table) {
                for (alias, dependency) in dependencies {
                    let dependency = if dependency.as_table().is_some_and(|table| {
                        table.get("workspace").and_then(Value::as_bool) == Some(true)
                    }) {
                        workspace
                            .get("workspace")
                            .and_then(Value::as_table)
                            .and_then(|table| table.get("dependencies"))
                            .and_then(Value::as_table)
                            .and_then(|table| table.get(alias))
                            .expect("workspace dependency")
                    } else {
                        dependency
                    };
                    let name = dependency_package(alias, dependency);
                    assert!(
                        !forbidden.contains(&name),
                        "{crate_name} has a forbidden {section} dependency: {alias} (package {name})",
                    );
                }
            }
        }
    };
    check(&package);
    if let Some(targets) = package.get("target").and_then(Value::as_table) {
        for target in targets.values() {
            check(target.as_table().expect("target dependency table"));
        }
    }
}

fn visit_crate_rust(directory: &Path, visitor: &mut impl FnMut(&Path, &str)) {
    for surface in ["src", "tests", "examples"] {
        let surface = directory.join(surface);
        if surface.is_dir() {
            visit_rust(&surface, visitor);
        }
    }
}

fn visit_rust(directory: &Path, visitor: &mut impl FnMut(&Path, &str)) {
    for entry in fs::read_dir(directory).expect("source directory") {
        let entry = entry.expect("source entry");
        let path = entry.path();
        if path.is_dir() {
            visit_rust(&path, visitor);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
            visitor(&path, &source);
        }
    }
}

fn path_starts_with(path: &[String], prefix: &[&str]) -> bool {
    path.len() >= prefix.len()
        && path
            .iter()
            .zip(prefix)
            .all(|(segment, expected)| segment.as_str() == *expected)
}

fn imports_runtime_session_handle(path: &[String]) -> bool {
    path_starts_with(path, &["agent_runtime", "runtime"])
        && (path.len() == 2 || matches!(path[2].as_str(), "self" | "*" | "SessionHandle"))
}

fn imports(source: &str, production_only: bool) -> Vec<Vec<String>> {
    let file = syn::parse_file(source).expect("Rust source syntax");
    let mut visitor = Imports {
        production_only,
        paths: Vec::new(),
    };
    visitor.visit_file(&file);
    visitor.paths
}

struct Imports {
    production_only: bool,
    paths: Vec<Vec<String>>,
}

impl<'ast> Visit<'ast> for Imports {
    fn visit_file(&mut self, file: &'ast syn::File) {
        if !self.production_only || !test_only(&file.attrs) {
            visit::visit_file(self, file);
        }
    }

    fn visit_item(&mut self, item: &'ast Item) {
        let attrs = match item {
            Item::Const(item) => &item.attrs,
            Item::Enum(item) => &item.attrs,
            Item::ExternCrate(item) => &item.attrs,
            Item::Fn(item) => &item.attrs,
            Item::ForeignMod(item) => &item.attrs,
            Item::Impl(item) => &item.attrs,
            Item::Macro(item) => &item.attrs,
            Item::Mod(item) => &item.attrs,
            Item::Static(item) => &item.attrs,
            Item::Struct(item) => &item.attrs,
            Item::Trait(item) => &item.attrs,
            Item::TraitAlias(item) => &item.attrs,
            Item::Type(item) => &item.attrs,
            Item::Union(item) => &item.attrs,
            Item::Use(item) => &item.attrs,
            _ => return,
        };
        if !self.production_only || !test_only(attrs) {
            visit::visit_item(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !self.production_only || !test_only(&item.attrs) {
            visit::visit_impl_item_fn(self, item);
        }
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if !self.production_only || !test_only(&item.attrs) {
            visit::visit_trait_item_fn(self, item);
        }
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        use_paths(&item.tree, &[], &mut self.paths);
    }
}

fn use_paths(tree: &UseTree, prefix: &[String], paths: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            let mut prefix = prefix.to_vec();
            prefix.push(path.ident.to_string());
            use_paths(&path.tree, &prefix, paths);
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                use_paths(tree, prefix, paths);
            }
        }
        UseTree::Name(name) => {
            let mut path = prefix.to_vec();
            path.push(name.ident.to_string());
            paths.push(path);
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.to_vec();
            path.push(rename.ident.to_string());
            paths.push(path);
        }
        UseTree::Glob(_) => {
            let mut path = prefix.to_vec();
            path.push("*".to_owned());
            paths.push(path);
        }
    }
}

fn test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<Meta>()
                .is_ok_and(|meta| cfg_without_test(&meta) == Some(false))
    })
}

// Unknown platform/feature predicates remain eligible for production checks.
fn cfg_without_test(meta: &Meta) -> Option<bool> {
    if meta.path().is_ident("test") {
        return Some(false);
    }
    let Meta::List(list) = meta else {
        return None;
    };
    let args = list
        .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .expect("cfg arguments");
    let values: Vec<_> = args.iter().map(cfg_without_test).collect();
    if list.path.is_ident("all") {
        if values.contains(&Some(false)) {
            Some(false)
        } else if values.iter().all(|value| *value == Some(true)) {
            Some(true)
        } else {
            None
        }
    } else if list.path.is_ident("any") {
        if values.contains(&Some(true)) {
            Some(true)
        } else if values.iter().all(|value| *value == Some(false)) {
            Some(false)
        } else {
            None
        }
    } else if list.path.is_ident("not") && values.len() == 1 {
        values[0].map(|value| !value)
    } else {
        None
    }
}

#[test]
fn import_checks_parse_grouped_renamed_and_nested_paths_without_scanning_text() {
    let paths = imports(
        r#"
        // use agent_runtime_core::event::RuntimeEvent;
        const DESCRIPTION: &str = "use smith_tui::*;";
        use agent_runtime_core::{event::{RuntimeEvent as Event, self}, ids::SessionId};
        pub use agent_runtime::{runtime::{SessionHandle as Handle}};
        fn nested() { use smith_tui::{app::App, *}; }
    "#,
        false,
    );
    assert_eq!(
        paths
            .iter()
            .filter(|path| path_starts_with(path, &["agent_runtime_core", "event"]))
            .count(),
        2
    );
    assert_eq!(
        paths
            .iter()
            .filter(|path| imports_runtime_session_handle(path))
            .count(),
        1
    );
    assert_eq!(
        paths
            .iter()
            .filter(|path| path_starts_with(path, &["smith_tui"]))
            .count(),
        2
    );
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.last().is_some_and(|segment| segment == "*"))
            .count(),
        1
    );
}

#[test]
fn production_import_checks_exclude_test_only_items_and_keep_platform_code() {
    let source = r#"
        #[cfg(test)] mod tests { use super::*; }
        #[cfg(all(test, unix))] fn test_helper() { use super::*; }
        #[cfg(test)] use fixtures::*;
        #[cfg(any(test, unix))] fn platform_code() { use platform::*; }
        #[cfg(not(test))] fn production() { use production::*; }
        struct Host;
        impl Host { #[cfg(test)] fn test_helper() { use super::*; } }
    "#;
    assert_eq!(
        imports(source, true),
        [
            vec!["platform".to_owned(), "*".to_owned()],
            vec!["production".to_owned(), "*".to_owned()]
        ]
    );
    assert_eq!(imports(source, false).len(), 6);
    assert!(imports("#![cfg(test)] use super::*;", true).is_empty());
}
