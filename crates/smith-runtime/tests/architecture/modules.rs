//! Native module manifests and their explicit composition root.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use syn::visit::{self, Visit};
use syn::{Item, Visibility};
use toml::{Table, Value};

use super::{dependency_package, manifest, test_only, visit_rust, workspace_root};

fn dependency_names(table: &Table) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dependencies) = table.get(section).and_then(Value::as_table) {
            names.extend(
                dependencies
                    .iter()
                    .map(|(alias, entry)| dependency_package(alias, entry).to_owned()),
            );
        }
    }
    if let Some(targets) = table.get("target").and_then(Value::as_table) {
        for target in targets.values().filter_map(Value::as_table) {
            names.extend(dependency_names(target));
        }
    }
    names
}

fn module_manifests(directory: &Path, output: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(directory).expect("module directory") {
        let path = entry.expect("module entry").path();
        if path.is_dir() {
            module_manifests(&path, output);
        } else if path.file_name().is_some_and(|name| name == "Cargo.toml") {
            output.push(path);
        }
    }
}

#[test]
fn module_crates_obey_boundaries_and_have_one_default_optional_feature() {
    let root = workspace_root();
    let cli = manifest(&root.join("crates/smith-cli/Cargo.toml"));
    let dependencies = cli["dependencies"].as_table().unwrap();
    let features = cli["features"].as_table().unwrap();
    let default = features["default"].as_array().unwrap();
    let mut paths = Vec::new();
    module_manifests(&root.join("crates/modules"), &mut paths);
    assert!(!paths.is_empty(), "the ported module crates must exist");
    let catalog = fs::read_to_string(root.join("crates/smith-cli/src/modules.rs")).unwrap();
    for path in paths {
        let module = manifest(&path);
        let name = module["package"]["name"].as_str().unwrap();
        let names = dependency_names(&module);
        assert!(
            names.contains("smith-module"),
            "{name} must depend on smith-module"
        );
        for forbidden in ["smith-runtime", "smith-config", "smith-tui", "smith-cli"] {
            assert!(
                !names.contains(forbidden),
                "{name} depends on forbidden {forbidden}"
            );
        }
        let (alias, dependency) = dependencies
            .iter()
            .find(|(alias, entry)| dependency_package(alias, entry) == name)
            .unwrap_or_else(|| panic!("{name} has no smith-cli dependency"));
        assert_eq!(
            dependency.get("optional").and_then(Value::as_bool),
            Some(true),
            "{name} must be optional"
        );
        let id = path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let feature = format!("module-{id}");
        let activation = format!("dep:{alias}");
        let enabling = features.iter().filter(|(_, values)| values.as_array().is_some_and(|values| {
            values.iter().any(|value| matches!(value.as_str(), Some(value) if value == activation || value == alias || value.starts_with(&format!("{alias}/"))))
        })).map(|(key, _)| key.as_str()).collect::<Vec<_>>();
        assert_eq!(
            enabling,
            [feature.as_str()],
            "{name} must be enabled by exactly {feature}"
        );
        assert_eq!(
            features[&feature].as_array().unwrap(),
            &[Value::String(activation)],
            "{name} feature must directly enable its optional dependency"
        );
        assert!(
            default.iter().any(|entry| entry.as_str() == Some(&feature)),
            "{name} must be in default features"
        );
        let gate = format!("#[cfg(feature = \"{feature}\")]");
        let entry = format!("module: Arc::new({}::", alias.replace('-', "_"));
        let body = catalog.split("pub(super) fn composition").next().unwrap();
        let offset = body
            .find(&entry)
            .unwrap_or_else(|| panic!("{name} missing from compiled-in list"));
        let prefix = &body[..offset];
        assert!(
            prefix
                .rsplit("CompiledModule {")
                .nth(1)
                .is_some_and(|before| before.trim_end().ends_with(&gate)),
            "{name} compiled entry lacks {feature} gate"
        );
    }
}

#[test]
fn compiled_module_list_has_one_owner_and_no_link_time_registration() {
    let root = workspace_root();
    let workspace = manifest(&root.join("Cargo.toml"));
    let mut owners = Vec::new();
    let mut entry_owners = BTreeSet::new();
    for member in workspace["workspace"]["members"].as_array().unwrap() {
        let directory = root.join(member.as_str().unwrap());
        let package = manifest(&directory.join("Cargo.toml"));
        let name = package["package"]["name"].as_str().unwrap();
        for forbidden in ["inventory", "linkme", "ctor"] {
            assert!(
                !dependency_names(&package).contains(forbidden),
                "{name} depends on forbidden registration crate {forbidden}"
            );
        }
        visit_rust(&directory.join("src"), &mut |path, source| {
            let relative = path.strip_prefix(&root).unwrap();
            if relative
                .components()
                .any(|part| matches!(part.as_os_str().to_str(), Some("tests" | "main_tests")))
                || path.file_name().is_some_and(|name| name == "tests.rs")
            {
                return;
            }
            let file = syn::parse_file(source).expect("Rust syntax");
            let mut entries = CompiledEntries::default();
            entries.visit_file(&file);
            if entries.0 > 0 {
                entry_owners.insert(relative.to_path_buf());
            }
            for item in file.items {
                if let Item::Fn(function) = item
                    && function.sig.ident == "compiled_modules"
                {
                    owners.push(path.strip_prefix(&root).unwrap().to_path_buf());
                    assert!(
                        matches!(function.vis, Visibility::Inherited),
                        "the build list belongs to the CLI composition root"
                    );
                }
            }
        });
    }
    assert_eq!(
        owners,
        [std::path::PathBuf::from("crates/smith-cli/src/modules.rs")]
    );
    assert_eq!(
        entry_owners,
        BTreeSet::from([std::path::PathBuf::from("crates/smith-cli/src/modules.rs")])
    );
    let fixture = manifest(&root.join("crates/test-support/module-fixture/Cargo.toml"));
    assert_eq!(fixture["package"]["publish"].as_bool(), Some(false));
    for member in workspace["workspace"]["members"].as_array().unwrap() {
        let package = manifest(&root.join(member.as_str().unwrap()).join("Cargo.toml"));
        for section in ["dependencies", "build-dependencies"] {
            assert!(
                package
                    .get(section)
                    .and_then(Value::as_table)
                    .is_none_or(|dependencies| {
                        dependencies.iter().all(|(alias, entry)| {
                            dependency_package(alias, entry) != "smith-test-module"
                        })
                    }),
                "the third-party fixture may only be linked by tests"
            );
        }
    }
}

#[derive(Default)]
struct CompiledEntries(usize);

impl<'ast> Visit<'ast> for CompiledEntries {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        if !test_only(&module.attrs) {
            visit::visit_item_mod(self, module);
        }
    }
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        if !test_only(&function.attrs) {
            visit::visit_item_fn(self, function);
        }
    }
    fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
        if expression
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "CompiledModule")
        {
            self.0 += 1;
        }
        visit::visit_expr_struct(self, expression);
    }
}
