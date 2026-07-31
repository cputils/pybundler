use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path};

use crate::bundler::{ImportedModuleBudget, ModuleData};
use crate::resolver::ModuleResolver;

pub(crate) fn ensure_parent_packages(
    module_name: &str,
    resolver: &ModuleResolver,
    module_map: &mut HashMap<String, ModuleData>,
    queue: &mut VecDeque<String>,
    import_budget: &mut ImportedModuleBudget,
    allow_sys_path_imports: bool,
) -> Result<bool, String> {
    let parts = module_name.split('.').collect::<Vec<_>>();
    if parts.len() < 2 {
        return Ok(false);
    }

    let mut changed = false;
    for i in 1..parts.len() {
        let parent = parts[..i].join(".");
        if let Some(existing) = module_map.get_mut(&parent) {
            if allow_sys_path_imports && !existing.allow_sys_path_imports {
                existing.allow_sys_path_imports = true;
                if !existing.synthetic {
                    queue.push_back(parent);
                }
                changed = true;
            }
            if !existing.is_package {
                break;
            }
            continue;
        }

        let Some(resolved) = resolver
            .resolve_module(&parent)
            .map_err(|err| format!("resolve parent package {:?}: {err}", parent))?
        else {
            break;
        };
        if resolved.from_sys_path && !allow_sys_path_imports {
            break;
        }
        let is_package = resolved.is_package;
        import_budget.track(&parent)?;
        module_map.insert(
            parent.clone(),
            ModuleData {
                name: parent.clone(),
                file_path: resolved.file_path,
                is_package,
                synthetic: resolved.synthetic,
                allow_sys_path_imports: allow_sys_path_imports || resolved.from_sys_path,
                source: Vec::new(),
                analysis: None,
            },
        );
        if !resolved.synthetic {
            queue.push_back(parent);
        }
        changed = true;
        if !is_package {
            break;
        }
    }

    Ok(changed)
}

pub(crate) fn module_name_from_path(
    project_root: &Path,
    file_path: &Path,
) -> Result<(String, bool), String> {
    let rel = file_path.strip_prefix(project_root).map_err(|_| {
        format!(
            "entry file {} must be inside project root {}",
            file_path.display(),
            project_root.display()
        )
    })?;

    let base = rel
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid module path for file {}", file_path.display()))?;
    if is_package_init(file_path, base) {
        let dir = rel.parent().unwrap_or_else(|| Path::new("."));
        if dir == Path::new(".") {
            return Err(format!(
                "cannot determine module name for root-level __init__.py: {}",
                file_path.display()
            ));
        }
        let module_name = rel_components_to_module_name(dir)?;
        return Ok((module_name, true));
    }

    let ext = Path::new(base).extension().and_then(|ext| ext.to_str());
    if !ext.is_some_and(|value| value.eq_ignore_ascii_case("py")) {
        return Err(format!(
            "python module path must end with .py: {}",
            file_path.display()
        ));
    }

    let mut path_no_ext = rel.to_path_buf();
    path_no_ext.set_extension("");
    let module_name = rel_components_to_module_name(&path_no_ext)?;
    if module_name.is_empty() || module_name == "." {
        return Err(format!(
            "invalid module path for file {}",
            file_path.display()
        ));
    }
    Ok((module_name, false))
}

fn is_package_init(file_path: &Path, base: &str) -> bool {
    if base == "__init__.py" {
        return true;
    }
    if !base.eq_ignore_ascii_case("__init__.py") {
        return false;
    }
    let Ok(expected) = file_path.with_file_name("__init__.py").canonicalize() else {
        return false;
    };
    expected == file_path
}

fn rel_components_to_module_name(path: &Path) -> Result<String, String> {
    let mut out = Vec::new();
    for component in path.components() {
        if let Component::Normal(part) = component {
            let value = part
                .to_str()
                .ok_or_else(|| "module path must be valid UTF-8".to_string())?;
            if !value.is_empty() {
                out.push(value.to_string());
            }
        }
    }
    Ok(out.join("."))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::module_name_from_path;

    #[test]
    fn does_not_treat_differently_cased_init_as_package_on_case_sensitive_fs() {
        let root =
            std::env::temp_dir().join(format!("pybundler-module-name-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create test directory");
        let entry = root.join("__INIT__.PY");
        std::fs::write(&entry, b"pass\n").expect("write entry module");

        let module = module_name_from_path(&root, &entry).expect("derive module name");

        std::fs::remove_file(entry).expect("remove entry module");
        std::fs::remove_dir(root).expect("remove test directory");
        assert_eq!(module, ("__INIT__".to_string(), false));
    }
}
