//! Keep shared client accounting independent of terminal libraries.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

#[test]
fn normal_dependency_closure_has_no_terminal_libraries() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("smith-client is a workspace crate");
    let output = Command::new(env!("CARGO"))
        // Not `--offline`: metadata covers every target platform, and a
        // checkout that built for one host has not downloaded the others.
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(workspace)
        .output()
        .expect("read Cargo dependency metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).expect("Cargo metadata JSON");
    let packages: BTreeMap<&str, &Value> = metadata["packages"]
        .as_array()
        .expect("Cargo packages")
        .iter()
        .map(|package| (package["id"].as_str().expect("package ID"), package))
        .collect();
    let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .expect("Cargo dependency graph")
        .iter()
        .map(|node| (node["id"].as_str().expect("node ID"), node))
        .collect();
    let client = packages
        .iter()
        .find(|(_, package)| {
            package["name"] == "smith-client"
                && Path::new(package["manifest_path"].as_str().expect("manifest path"))
                    == Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
        })
        .map(|(id, _)| *id)
        .expect("smith-client package");
    let mut pending = vec![client];
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let name = packages[id]["name"].as_str().expect("package name");
        assert!(
            !matches!(name, "ratatui" | "crossterm"),
            "smith-client's normal dependency closure includes {name}",
        );
        for dependency in nodes[id]["deps"].as_array().expect("node dependencies") {
            // Cargo represents normal edges with a null kind. Include every
            // target condition, but do not follow dev or build dependencies.
            if dependency["dep_kinds"]
                .as_array()
                .expect("dependency kinds")
                .iter()
                .any(|kind| kind["kind"].is_null())
            {
                pending.push(dependency["pkg"].as_str().expect("dependency package ID"));
            }
        }
    }
}
