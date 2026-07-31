//! Workspace dependency-direction checks that run on every supported host.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

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
