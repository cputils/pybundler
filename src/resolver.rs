use std::cell::RefCell;
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::codegen::module_package_name;

/// Separator used in virtual file paths to indicate a zip archive entry.
/// For example: `/path/to/pkg.zip::foo/bar.py`
const ZIP_SEPARATOR: &str = "::";

#[derive(Clone, Debug)]
pub(crate) struct ResolvedModule {
    pub name: String,
    pub file_path: PathBuf,
    pub is_package: bool,
}

#[derive(Debug)]
pub(crate) struct ModuleResolver {
    dir_roots: Vec<PathBuf>,
    zip_roots: Vec<PathBuf>,
    zip_name_cache: RefCell<Vec<HashSet<String>>>,
}

impl ModuleResolver {
    pub(crate) fn new(search_roots: Vec<PathBuf>) -> Self {
        let mut dir_roots = Vec::new();
        let mut zip_roots = Vec::new();
        for root in search_roots {
            if root.as_os_str().is_empty() {
                continue;
            }
            let clean = root.components().collect::<PathBuf>();
            if clean.is_dir() {
                dir_roots.push(clean);
            } else if is_importable_zip(&clean) {
                zip_roots.push(clean);
            }
        }
        Self {
            dir_roots,
            zip_roots,
            zip_name_cache: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn resolve_module(
        &self,
        module_name: &str,
    ) -> Result<Option<ResolvedModule>, String> {
        let module_name = module_name.trim();
        if module_name.is_empty() {
            return Ok(None);
        }
        let parts = module_name.split('.').collect::<Vec<_>>();
        if parts.iter().any(|part| part.is_empty()) {
            return Err(format!("invalid module name {:?}", module_name));
        }

        for root in &self.dir_roots {
            let mut base = root.clone();
            for part in &parts {
                base.push(part);
            }

            let init_file = base.join("__init__.py");
            if regular_file_exists(&init_file)? {
                return Ok(Some(ResolvedModule {
                    name: module_name.to_string(),
                    file_path: init_file,
                    is_package: true,
                }));
            }

            let module_file = base.with_extension("py");
            if regular_file_exists(&module_file)? {
                return Ok(Some(ResolvedModule {
                    name: module_name.to_string(),
                    file_path: module_file,
                    is_package: false,
                }));
            }
        }

        let internal_path = parts.join("/");
        if let Some(result) = self.resolve_in_zips(module_name, &internal_path)? {
            return Ok(Some(result));
        }

        Ok(None)
    }

    fn resolve_in_zips(
        &self,
        module_name: &str,
        internal_path: &str,
    ) -> Result<Option<ResolvedModule>, String> {
        let mut cache = self.zip_name_cache.borrow_mut();
        for (idx, zip_root) in self.zip_roots.iter().enumerate() {
            if cache.len() <= idx {
                let names = match File::open(zip_root)
                    .ok()
                    .and_then(|file| zip::ZipArchive::new(file).ok())
                {
                    Some(archive) => archive.file_names().map(|s| s.to_string()).collect(),
                    None => HashSet::new(),
                };
                cache.push(names);
            }

            let init_path = format!("{internal_path}/__init__.py");
            if cache[idx].contains(&init_path) {
                let file_path = format!("{}{ZIP_SEPARATOR}{init_path}", zip_root.display());
                return Ok(Some(ResolvedModule {
                    name: module_name.to_string(),
                    file_path: PathBuf::from(file_path),
                    is_package: true,
                }));
            }

            let module_path = format!("{internal_path}.py");
            if cache[idx].contains(&module_path) {
                let file_path = format!("{}{ZIP_SEPARATOR}{module_path}", zip_root.display());
                return Ok(Some(ResolvedModule {
                    name: module_name.to_string(),
                    file_path: PathBuf::from(file_path),
                    is_package: false,
                }));
            }
        }
        Ok(None)
    }
}

/// Returns `true` if `path` points to a regular file that looks like a
/// Python-importable archive (`.zip` or `.egg`).
pub(crate) fn is_importable_zip(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| matches!(ext, "zip" | "egg"))
}

fn regular_file_exists(path: &Path) -> Result<bool, String> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("stat {}: {err}", path.display())),
    };
    Ok(meta.is_file())
}

/// Reads the source code of a module, handling both regular files and
/// zip archive entries (paths containing `::`).
pub(crate) fn read_module_source(file_path: &Path) -> Result<Vec<u8>, String> {
    let path_str = file_path.to_string_lossy();
    if let Some(pos) = path_str.find(ZIP_SEPARATOR) {
        let zip_path = Path::new(&path_str[..pos]);
        let entry_path = &path_str[pos + ZIP_SEPARATOR.len()..];
        let file = File::open(zip_path)
            .map_err(|err| format!("open zip file {}: {err}", zip_path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|err| format!("read zip {}: {err}", zip_path.display()))?;
        let mut entry = archive
            .by_name(entry_path)
            .map_err(|err| format!("entry {entry_path} in {}: {err}", zip_path.display()))?;
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|err| format!("read {entry_path} from {}: {err}", zip_path.display()))?;
        Ok(buf)
    } else {
        std::fs::read(file_path).map_err(|err| format!("read file {}: {err}", file_path.display()))
    }
}

pub(crate) fn resolve_relative_module_name(
    current_module: &str,
    current_is_package: bool,
    raw_module: &str,
    level: usize,
) -> Result<String, String> {
    if level == 0 {
        return Ok(raw_module.trim().to_string());
    }
    let package_name = module_package_name(current_module, current_is_package);
    if package_name.is_empty() {
        return Err("relative import requires a package context".to_string());
    }

    let parts = package_name.split('.').collect::<Vec<_>>();
    if level - 1 > parts.len() {
        return Err("relative import goes beyond top-level package".to_string());
    }
    let base = parts[..parts.len() - (level - 1)].join(".");
    let name = raw_module.trim();
    if name.is_empty() {
        return Ok(base);
    }
    if base.is_empty() {
        return Ok(name.to_string());
    }
    Ok(format!("{base}.{name}"))
}
