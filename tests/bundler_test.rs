use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use pybundler::{BundleOptions, BundledModule, bundle_file};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct BundleScenario {
    entry: String,
    external: Vec<String>,
    #[serde(rename = "maxImportedModules")]
    max_imported_modules: usize,
    #[serde(rename = "shouldFail")]
    should_fail: bool,
    #[serde(rename = "errorContains")]
    error_contains: String,
    #[serde(rename = "mustIncludeModules")]
    must_include_modules: Vec<String>,
    #[serde(rename = "mustExcludeModules")]
    must_exclude_modules: Vec<String>,
    #[serde(rename = "mustBeSynthetic")]
    must_be_synthetic: Vec<String>,
    #[serde(rename = "mustNotBeSynthetic")]
    must_not_be_synthetic: Vec<String>,
    #[serde(rename = "mustContain")]
    must_contain: Vec<String>,
    #[serde(rename = "mustNotContain")]
    must_not_contain: Vec<String>,
    #[serde(rename = "mustContainCount")]
    must_contain_count: HashMap<String, usize>,
    interpreter: Vec<String>,
}

impl BundleScenario {
    fn with_defaults(mut self) -> Self {
        if self.entry.trim().is_empty() {
            self.entry = "main.py".to_string();
        }
        if self.max_imported_modules == 0 {
            self.max_imported_modules = 2048;
        }
        self
    }
}

#[test]
fn test_bundle_file() {
    for scenario_name in discover_scenario_names() {
        let scenario_path = testdata_root().join(&scenario_name);
        let project_root = scenario_project_root(&scenario_path);
        let scenario = load_scenario(&scenario_path);
        execute_scenario(&scenario_path, &project_root, &scenario);
    }
}

fn execute_scenario(scenario_path: &Path, project_root: &Path, scenario: &BundleScenario) {
    let interpreter: Vec<String> = scenario
        .interpreter
        .iter()
        .map(|name| {
            let dir = project_root.join(name);
            let script_path = scenario_path.join(name);
            if cfg!(windows) {
                let p = script_path.with_extension("bat");
                fs::write(&p, format!("@echo {}\n", dir.display())).unwrap();
                p
            } else {
                let p = script_path.with_extension("sh");
                fs::write(&p, format!("#!/bin/sh\necho {}\n", dir.display())).unwrap();
                std::process::Command::new("chmod")
                    .args(["u+x", &p.display().to_string()])
                    .status()
                    .unwrap();
                p
            }
            .display()
            .to_string()
        })
        .collect();

    let result = bundle_file(
        &project_root.join(&scenario.entry).display().to_string(),
        BundleOptions {
            external: scenario.external.clone(),
            ignore_comment_literal: "no-bundle".to_string(),
            max_imported_modules: scenario.max_imported_modules,
            interpreter,
        },
    );

    if scenario.should_fail {
        let err = result.expect_err("expected bundle_file to fail");
        if !scenario.error_contains.is_empty() {
            assert!(
                err.contains(&scenario.error_contains),
                "expected error to contain {:?}, got: {}",
                scenario.error_contains,
                err
            );
        }
        return;
    }

    let result = result.unwrap_or_else(|err| panic!("bundle_file returned error: {err}"));
    write_bundled_snapshot(scenario_path, project_root, &result.code);

    let modules = bundled_module_map(&result.bundled_module_list);
    for name in &scenario.must_include_modules {
        assert!(
            modules.contains_key(name),
            "expected bundled module {:?} to exist",
            name
        );
    }
    for name in &scenario.must_exclude_modules {
        assert!(
            !modules.contains_key(name),
            "expected bundled module {:?} to be excluded",
            name
        );
    }
    for name in &scenario.must_be_synthetic {
        let module = modules
            .get(name)
            .unwrap_or_else(|| panic!("expected bundled module {:?} to exist", name));
        assert!(
            module.synthetic,
            "expected module {:?} to be synthetic",
            name
        );
    }
    for name in &scenario.must_not_be_synthetic {
        let module = modules
            .get(name)
            .unwrap_or_else(|| panic!("expected bundled module {:?} to exist", name));
        assert!(
            !module.synthetic,
            "expected module {:?} to be non-synthetic",
            name
        );
    }

    for token in &scenario.must_contain {
        assert!(
            result.code.contains(token),
            "expected bundled code to contain {:?}",
            token
        );
    }
    for token in &scenario.must_not_contain {
        assert!(
            !result.code.contains(token),
            "expected bundled code not to contain {:?}",
            token
        );
    }
    for (token, expected) in &scenario.must_contain_count {
        let actual = result.code.matches(token).count();
        assert_eq!(
            actual, *expected,
            "expected {:?} to appear {} times, got {}",
            token, expected, actual
        );
    }
}

fn discover_scenario_names() -> Vec<String> {
    let entries = fs::read_dir(testdata_root()).expect("read testdata directory");
    let mut names = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            Some(name)
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn load_scenario(scenario_path: &Path) -> BundleScenario {
    let case_path = scenario_path.join("case.json");
    let data = match fs::read_to_string(&case_path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return BundleScenario::default().with_defaults();
        }
        Err(err) => panic!("read {}: {err}", case_path.display()),
    };
    serde_json::from_str::<BundleScenario>(&data)
        .unwrap_or_else(|err| panic!("parse {}: {err}", case_path.display()))
        .with_defaults()
}

fn scenario_project_root(scenario_path: &Path) -> PathBuf {
    scenario_path.canonicalize().unwrap_or_else(|err| {
        panic!(
            "resolve scenario path for {}: {err}",
            scenario_path.display()
        )
    })
}

fn write_bundled_snapshot(scenario_path: &Path, project_root: &Path, code: &str) {
    let normalized = normalize_bundle_code(code, project_root);
    let out_path = scenario_path.join("bundled.py");

    if let Ok(current) = fs::read_to_string(&out_path)
        && normalize_newlines(&current) == normalized
    {
        return;
    }
    fs::write(&out_path, normalized).unwrap_or_else(|err| {
        panic!(
            "write bundled snapshot for {}: {err}",
            scenario_path.display()
        )
    });
}

fn normalize_bundle_code(code: &str, project_root: &Path) -> String {
    let mut out = normalize_newlines(code);
    let clean_root = project_root.components().collect::<PathBuf>();
    let clean_root_str = clean_root.display().to_string();
    let escaped_root = clean_root_str.replace('\\', "\\\\");
    out = out.replace(&escaped_root, "${PROJECT_ROOT}");
    out.replace(&clean_root_str, "${PROJECT_ROOT}")
}

fn bundled_module_map(modules: &[BundledModule]) -> HashMap<String, BundledModule> {
    let mut out = HashMap::new();
    for module in modules {
        out.insert(module.name.clone(), module.clone());
    }
    out
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n")
}

fn testdata_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}
