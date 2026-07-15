use std::collections::{HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};

use crate::bundler::{ImportedModuleBudget, ModuleData};
use crate::resolver::ModuleResolver;

pub(crate) fn ensure_parent_packages(
    module_name: &str,
    resolver: &ModuleResolver,
    module_map: &mut HashMap<String, ModuleData>,
    queue: &mut VecDeque<String>,
    import_budget: &mut ImportedModuleBudget,
) -> Result<bool, String> {
    let parts = module_name.split('.').collect::<Vec<_>>();
    if parts.len() < 2 {
        return Ok(false);
    }

    let mut changed = false;
    for i in 1..parts.len() {
        let parent = parts[..i].join(".");
        if module_map.contains_key(&parent) {
            continue;
        }

        if let Some(resolved) = resolver
            .resolve_module(&parent)
            .map_err(|err| format!("resolve parent package {:?}: {err}", parent))?
        {
            import_budget.track(&parent)?;
            module_map.insert(
                parent.clone(),
                ModuleData {
                    name: parent.clone(),
                    file_path: resolved.file_path,
                    is_package: true,
                    synthetic: false,
                    source: Vec::new(),
                    analysis: None,
                },
            );
            queue.push_back(parent);
            changed = true;
            continue;
        }

        import_budget.track(&parent)?;
        module_map.insert(
            parent.clone(),
            ModuleData {
                name: parent.clone(),
                file_path: PathBuf::from(format!("<synthetic:{parent}>")),
                is_package: true,
                synthetic: true,
                source: Vec::new(),
                analysis: None,
            },
        );
        changed = true;
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
    if base.eq_ignore_ascii_case("__init__.py") {
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
