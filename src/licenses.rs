use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::bundler::ModuleData;

pub(crate) struct LicenseComment {
    pub package_name: String,
    pub version: String,
    pub text: String,
    pub target_module: String,
}

pub(crate) fn collect_license_comments(
    sys_paths: &[PathBuf],
    module_map: &HashMap<String, ModuleData>,
    sorted_modules: &[String],
) -> Vec<LicenseComment> {
    if sys_paths.is_empty() || sorted_modules.is_empty() {
        return Vec::new();
    }

    let mut seen_packages = HashSet::new();
    let mut comments = Vec::new();

    for sys_path in sys_paths {
        let dist_infos = match collect_dist_info_dirs(sys_path) {
            Ok(dirs) => dirs,
            Err(_) => continue,
        };

        for dist_info in &dist_infos {
            let (pkg_name, version) = match read_metadata(dist_info) {
                Some(v) => v,
                None => continue,
            };

            if !seen_packages.insert(pkg_name.clone()) {
                continue;
            }

            let record_paths = match read_record(dist_info) {
                Some(v) => v,
                None => continue,
            };

            let record_set: HashSet<&str> = record_paths.iter().map(|s| s.as_str()).collect();

            let target_module =
                match find_earliest_match(sys_path, &record_set, module_map, sorted_modules) {
                    Some(m) => m,
                    None => continue,
                };

            let license_text = match find_license_text(sys_path, &record_paths) {
                Some(t) => t,
                None => continue,
            };

            comments.push(LicenseComment {
                package_name: pkg_name,
                version,
                text: license_text,
                target_module: target_module.to_string(),
            });
        }
    }

    comments.sort_by_key(|c| {
        sorted_modules
            .iter()
            .position(|m| m == &c.target_module)
            .unwrap_or(usize::MAX)
    });

    comments
}

fn collect_dist_info_dirs(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut dirs = Vec::new();
    let entries = match fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return Ok(dirs),
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".dist-info") {
                dirs.push(entry.path());
            }
        }
    }
    Ok(dirs)
}

fn read_metadata(dist_info: &Path) -> Option<(String, String)> {
    let content = fs::read_to_string(dist_info.join("METADATA")).ok()?;
    let mut name = None;
    let mut version = None;
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("Name: ") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("Version: ") {
            version = Some(value.trim().to_string());
        }
        if name.is_some() && version.is_some() {
            break;
        }
    }
    Some((name?, version?))
}

fn read_record(dist_info: &Path) -> Option<Vec<String>> {
    let content = fs::read_to_string(dist_info.join("RECORD")).ok()?;
    let mut paths = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(path) = line.split(',').next() {
            let path = path.trim();
            if !path.is_empty() {
                paths.push(path.to_string());
            }
        }
    }
    Some(paths)
}

fn find_earliest_match<'a>(
    sys_path: &Path,
    record_set: &HashSet<&str>,
    module_map: &'a HashMap<String, ModuleData>,
    sorted_modules: &'a [String],
) -> Option<&'a str> {
    for module_name in sorted_modules {
        let Some(mod_data) = module_map.get(module_name) else {
            continue;
        };
        let Ok(rel) = mod_data.file_path.strip_prefix(sys_path) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if record_set.contains(rel_str.as_str()) {
            return Some(module_name.as_str());
        }
    }
    None
}

fn read_file_backslashreplace(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let mut result = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        result.push_str(chunk.valid());
        for &b in chunk.invalid() {
            result.push_str(&format!("\\x{:02x}", b));
        }
    }
    Some(result)
}

fn find_license_text(sys_path: &Path, record_paths: &[String]) -> Option<String> {
    for path_str in record_paths {
        let name = Path::new(path_str).file_name()?;
        let name = name.to_str()?;
        if !is_license_filename(name) {
            continue;
        }
        let full_path = sys_path.join(path_str);
        if full_path.is_file() {
            return read_file_backslashreplace(&full_path);
        }
    }
    None
}

fn is_license_filename(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("license")
        || lower.starts_with("licence")
        || lower == "notice"
        || lower.starts_with("notice.")
        || lower.starts_with("authors")
}
