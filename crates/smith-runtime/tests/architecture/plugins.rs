//! Content bundles stay below runtime and terminal composition.

use std::collections::BTreeSet;

use toml::{Table, Value};

use super::{dependency_package, manifest, workspace_root};

fn dependencies(table: &Table, workspace: &Table) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(entries) = table.get(section).and_then(Value::as_table) {
            for (alias, entry) in entries {
                let resolved = if entry.get("workspace").and_then(Value::as_bool) == Some(true) {
                    workspace.get(alias).unwrap_or(entry)
                } else {
                    entry
                };
                names.insert(dependency_package(alias, resolved).to_owned());
            }
        }
    }
    if let Some(targets) = table.get("target").and_then(Value::as_table) {
        for target in targets.values().filter_map(Value::as_table) {
            names.extend(dependencies(target, workspace));
        }
    }
    names
}

#[test]
fn plugins_depend_only_on_configuration_and_never_on_git_libraries() {
    let root = workspace_root();
    let workspace = manifest(&root.join("Cargo.toml"));
    let shared = workspace["workspace"]["dependencies"].as_table().unwrap();
    let members = workspace["workspace"]["members"].as_array().unwrap();
    let mut packages = Vec::new();
    for member in members {
        packages.push(manifest(
            &root.join(member.as_str().unwrap()).join("Cargo.toml"),
        ));
    }
    let workspace_names = packages
        .iter()
        .map(|package| package["package"]["name"].as_str().unwrap().to_owned())
        .collect::<BTreeSet<_>>();
    for package in &packages {
        let name = package["package"]["name"].as_str().unwrap();
        let names = dependencies(package, shared);
        for forbidden in ["git2", "gix"] {
            assert!(
                !names.contains(forbidden),
                "{name} must fetch through the user's git, not {forbidden}"
            );
        }
        if name == "smith-plugin" {
            assert_eq!(
                names
                    .intersection(&workspace_names)
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["smith-config".to_owned()])
            );
        }
        if name == "smith-tui" {
            assert!(
                !names.contains("smith-plugin"),
                "plugin inspection belongs to the host, not the terminal"
            );
        }
    }
}
