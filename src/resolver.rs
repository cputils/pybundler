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
    pub synthetic: bool,
    pub from_sys_path: bool,
}

#[derive(Debug)]
enum SearchRoot {
    Directory {
        path: PathBuf,
        from_sys_path: bool,
    },
    Zip {
        path: PathBuf,
        prefix: String,
        from_sys_path: bool,
        names: RefCell<Option<HashSet<String>>>,
    },
}

#[derive(Clone, Debug)]
enum SearchLocation {
    Directory {
        path: PathBuf,
        from_sys_path: bool,
    },
    Zip {
        root_index: usize,
        prefix: String,
        from_sys_path: bool,
    },
}

#[derive(Debug)]
enum ComponentResolution {
    Module {
        file_path: PathBuf,
        from_sys_path: bool,
    },
    Package {
        file_path: PathBuf,
        location: SearchLocation,
        from_sys_path: bool,
    },
    Namespace {
        locations: Vec<SearchLocation>,
    },
    Unbundleable,
}

#[derive(Debug)]
pub(crate) struct ModuleResolver {
    roots: Vec<SearchRoot>,
}

impl ModuleResolver {
    pub(crate) fn new(project_root: PathBuf, sys_path_roots: Vec<PathBuf>) -> Self {
        let mut roots = Vec::new();
        let mut seen = HashSet::new();
        for (root, from_sys_path) in std::iter::once((project_root, false))
            .chain(sys_path_roots.into_iter().map(|path| (path, true)))
        {
            if root.as_os_str().is_empty() {
                continue;
            }
            let clean = root.components().collect::<PathBuf>();
            if !seen.insert(clean.clone()) {
                continue;
            }
            if clean.is_dir() {
                roots.push(SearchRoot::Directory {
                    path: clean,
                    from_sys_path,
                });
            } else if let Some((path, prefix)) = split_importable_zip_path(&clean) {
                roots.push(SearchRoot::Zip {
                    path,
                    prefix,
                    from_sys_path,
                    names: RefCell::new(None),
                });
            }
        }
        Self { roots }
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

        let mut locations = self.root_locations();
        for (index, part) in parts.iter().enumerate() {
            let is_final = index + 1 == parts.len();
            let Some(resolved) = self.find_component(&locations, part)? else {
                return Ok(None);
            };
            match resolved {
                ComponentResolution::Module {
                    file_path,
                    from_sys_path,
                } if is_final => {
                    return Ok(Some(ResolvedModule {
                        name: module_name.to_string(),
                        file_path,
                        is_package: false,
                        synthetic: false,
                        from_sys_path,
                    }));
                }
                ComponentResolution::Module { .. } => return Ok(None),
                ComponentResolution::Package {
                    file_path,
                    location: _,
                    from_sys_path,
                } if is_final => {
                    return Ok(Some(ResolvedModule {
                        name: module_name.to_string(),
                        file_path,
                        is_package: true,
                        synthetic: false,
                        from_sys_path,
                    }));
                }
                ComponentResolution::Package { location, .. } => {
                    locations = vec![location];
                }
                ComponentResolution::Namespace {
                    locations: namespace_locations,
                } if is_final => {
                    let from_sys_path = namespace_locations
                        .iter()
                        .all(SearchLocation::is_from_sys_path);
                    return Ok(Some(ResolvedModule {
                        name: module_name.to_string(),
                        file_path: PathBuf::from(format!("<namespace:{module_name}>")),
                        is_package: true,
                        synthetic: true,
                        from_sys_path,
                    }));
                }
                ComponentResolution::Namespace {
                    locations: namespace_locations,
                } => {
                    locations = namespace_locations;
                }
                ComponentResolution::Unbundleable => return Ok(None),
            }
        }
        Ok(None)
    }

    fn root_locations(&self) -> Vec<SearchLocation> {
        self.roots
            .iter()
            .enumerate()
            .map(|(root_index, root)| match root {
                SearchRoot::Directory {
                    path,
                    from_sys_path,
                } => SearchLocation::Directory {
                    path: path.clone(),
                    from_sys_path: *from_sys_path,
                },
                SearchRoot::Zip {
                    prefix,
                    from_sys_path,
                    ..
                } => SearchLocation::Zip {
                    root_index,
                    prefix: prefix.clone(),
                    from_sys_path: *from_sys_path,
                },
            })
            .collect()
    }

    fn find_component(
        &self,
        locations: &[SearchLocation],
        name: &str,
    ) -> Result<Option<ComponentResolution>, String> {
        let mut namespace_locations = Vec::new();
        for location in locations {
            match location {
                SearchLocation::Directory {
                    path,
                    from_sys_path,
                } => {
                    let package_dir = path.join(name);
                    if extension_module_exists(&package_dir, "__init__") {
                        return Ok(Some(ComponentResolution::Unbundleable));
                    }
                    let init_file = package_dir.join("__init__.py");
                    if regular_file_exists(&init_file)? {
                        return Ok(Some(ComponentResolution::Package {
                            file_path: init_file,
                            location: SearchLocation::Directory {
                                path: package_dir,
                                from_sys_path: *from_sys_path,
                            },
                            from_sys_path: *from_sys_path,
                        }));
                    }
                    if regular_file_exists(&package_dir.join("__init__.pyc"))? {
                        return Ok(Some(ComponentResolution::Unbundleable));
                    }

                    if extension_module_exists(path, name) {
                        return Ok(Some(ComponentResolution::Unbundleable));
                    }
                    let module_file = path.join(name).with_extension("py");
                    if regular_file_exists(&module_file)? {
                        return Ok(Some(ComponentResolution::Module {
                            file_path: module_file,
                            from_sys_path: *from_sys_path,
                        }));
                    }
                    if regular_file_exists(&path.join(name).with_extension("pyc"))? {
                        return Ok(Some(ComponentResolution::Unbundleable));
                    }

                    if package_dir.is_dir() {
                        namespace_locations.push(SearchLocation::Directory {
                            path: package_dir,
                            from_sys_path: *from_sys_path,
                        });
                    }
                }
                SearchLocation::Zip {
                    root_index,
                    prefix,
                    from_sys_path,
                } => {
                    let internal_base = join_zip_path(prefix, name);
                    let init_path = format!("{internal_base}/__init__.py");
                    if self.zip_contains(*root_index, &init_path)? {
                        let zip_path = self.zip_path(*root_index)?;
                        return Ok(Some(ComponentResolution::Package {
                            file_path: PathBuf::from(format!(
                                "{}{ZIP_SEPARATOR}{init_path}",
                                zip_path.display()
                            )),
                            location: SearchLocation::Zip {
                                root_index: *root_index,
                                prefix: internal_base,
                                from_sys_path: *from_sys_path,
                            },
                            from_sys_path: *from_sys_path,
                        }));
                    }
                    if self.zip_contains(*root_index, &format!("{internal_base}/__init__.pyc"))? {
                        return Ok(Some(ComponentResolution::Unbundleable));
                    }

                    let module_path = format!("{internal_base}.py");
                    if self.zip_contains(*root_index, &module_path)? {
                        let zip_path = self.zip_path(*root_index)?;
                        return Ok(Some(ComponentResolution::Module {
                            file_path: PathBuf::from(format!(
                                "{}{ZIP_SEPARATOR}{module_path}",
                                zip_path.display()
                            )),
                            from_sys_path: *from_sys_path,
                        }));
                    }
                    if self.zip_contains(*root_index, &format!("{internal_base}.pyc"))? {
                        return Ok(Some(ComponentResolution::Unbundleable));
                    }

                    if self.zip_has_prefix(*root_index, &format!("{internal_base}/"))? {
                        namespace_locations.push(SearchLocation::Zip {
                            root_index: *root_index,
                            prefix: internal_base,
                            from_sys_path: *from_sys_path,
                        });
                    }
                }
            }
        }

        if namespace_locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(ComponentResolution::Namespace {
                locations: namespace_locations,
            }))
        }
    }

    fn zip_path(&self, root_index: usize) -> Result<&Path, String> {
        match self.roots.get(root_index) {
            Some(SearchRoot::Zip { path, .. }) => Ok(path),
            _ => Err("internal error: invalid zip search root".to_string()),
        }
    }

    fn zip_contains(&self, root_index: usize, name: &str) -> Result<bool, String> {
        Ok(self.zip_names(root_index)?.contains(name))
    }

    fn zip_has_prefix(&self, root_index: usize, prefix: &str) -> Result<bool, String> {
        Ok(self
            .zip_names(root_index)?
            .iter()
            .any(|name| name.starts_with(prefix)))
    }

    fn zip_names(&self, root_index: usize) -> Result<std::cell::Ref<'_, HashSet<String>>, String> {
        let Some(SearchRoot::Zip { path, names, .. }) = self.roots.get(root_index) else {
            return Err("internal error: invalid zip search root".to_string());
        };
        if names.borrow().is_none() {
            let loaded = File::open(path)
                .ok()
                .and_then(|file| zip::ZipArchive::new(file).ok())
                .map(|archive| archive.file_names().map(ToString::to_string).collect())
                .unwrap_or_default();
            *names.borrow_mut() = Some(loaded);
        }
        Ok(std::cell::Ref::map(names.borrow(), |value| {
            value.as_ref().expect("zip name cache must be populated")
        }))
    }
}

impl SearchLocation {
    fn is_from_sys_path(&self) -> bool {
        match self {
            Self::Directory { from_sys_path, .. } | Self::Zip { from_sys_path, .. } => {
                *from_sys_path
            }
        }
    }
}

fn join_zip_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Returns `true` if `path` points to a ZIP archive usable as a Python path
/// entry. CPython's zip importer does not require a particular extension.
pub(crate) fn is_importable_zip(path: &Path) -> bool {
    split_importable_zip_path(path).is_some()
}

fn split_importable_zip_path(path: &Path) -> Option<(PathBuf, String)> {
    for candidate in path.ancestors() {
        if !candidate.is_file()
            || File::open(candidate)
                .ok()
                .and_then(|file| zip::ZipArchive::new(file).ok())
                .is_none()
        {
            continue;
        }
        let prefix = path
            .strip_prefix(candidate)
            .ok()?
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .filter(|component| !component.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        return Some((candidate.to_path_buf(), prefix));
    }
    None
}

fn regular_file_exists(path: &Path) -> Result<bool, String> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("stat {}: {err}", path.display())),
    };
    Ok(meta.is_file())
}

fn extension_module_exists(directory: &Path, stem: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        if !entry.file_type().is_ok_and(|file_type| file_type.is_file()) {
            return false;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == format!("{stem}.so") || name == format!("{stem}.pyd") {
            return true;
        }
        let Some(tag) = name.strip_prefix(&format!("{stem}.")) else {
            return false;
        };
        (tag.ends_with(".so") && (tag.starts_with("cpython-") || tag == "abi3.so"))
            || (tag.ends_with(".pyd") && (tag.starts_with("cp") || tag == "abi3.pyd"))
    })
}

/// Reads the source code of a module, handling both regular files and
/// zip archive entries (paths containing `::`).
pub(crate) fn read_module_source(file_path: &Path) -> Result<Vec<u8>, String> {
    let path_str = file_path.to_string_lossy();
    let bytes = if let Some(pos) = path_str.find(ZIP_SEPARATOR) {
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
        buf
    } else {
        std::fs::read(file_path)
            .map_err(|err| format!("read file {}: {err}", file_path.display()))?
    };
    decode_python_source(&bytes, file_path).map(String::into_bytes)
}

fn decode_python_source(bytes: &[u8], file_path: &Path) -> Result<String, String> {
    let has_utf8_bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    let source = if has_utf8_bom { &bytes[3..] } else { bytes };
    let encoding = find_source_encoding(source).unwrap_or("utf-8");
    let normalized = encoding.to_ascii_lowercase().replace('_', "-");

    if has_utf8_bom && !matches!(normalized.as_str(), "utf-8" | "utf8" | "utf-8-sig") {
        return Err(format!(
            "encoding problem in {}: UTF-8 BOM conflicts with {encoding}",
            file_path.display()
        ));
    }

    match normalized.as_str() {
        "utf-8" | "utf8" | "utf-8-sig" => std::str::from_utf8(source)
            .map(ToString::to_string)
            .map_err(|err| format!("decode {} as UTF-8: {err}", file_path.display())),
        "ascii" | "us-ascii" => {
            if let Some((index, _)) = source.iter().enumerate().find(|(_, byte)| **byte > 0x7f) {
                return Err(format!(
                    "decode {} as ASCII: non-ASCII byte at offset {index}",
                    file_path.display()
                ));
            }
            Ok(source.iter().map(|byte| char::from(*byte)).collect())
        }
        "latin-1" | "latin1" | "iso-8859-1" | "iso8859-1" => {
            Ok(source.iter().map(|byte| char::from(*byte)).collect())
        }
        _ => Err(format!(
            "unsupported source encoding {encoding:?} in {}",
            file_path.display()
        )),
    }
}

fn find_source_encoding(bytes: &[u8]) -> Option<&str> {
    let first_end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(bytes.len());
    let first_line = &bytes[..first_end];
    if let Some(encoding) = find_encoding_in_line(first_line) {
        return Some(encoding);
    }
    if !first_line
        .iter()
        .copied()
        .find(|byte| !matches!(byte, b' ' | b'\t' | b'\x0c'))
        .is_none_or(|byte| matches!(byte, b'#' | b'\r'))
    {
        return None;
    }
    if first_end == bytes.len() {
        return None;
    }
    let second_start = first_end.saturating_add(1);
    let second_end = bytes[second_start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |index| second_start + index);
    find_encoding_in_line(&bytes[second_start..second_end])
}

fn find_encoding_in_line(line: &[u8]) -> Option<&str> {
    let comment = line.iter().position(|byte| *byte == b'#')?;
    let lower = line[comment..]
        .iter()
        .map(u8::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let coding = lower.windows(6).position(|window| window == b"coding")? + 6;
    let mut index = coding;
    while matches!(lower.get(index), Some(b' ' | b'\t')) {
        index += 1;
    }
    if !matches!(lower.get(index), Some(b':' | b'=')) {
        return None;
    }
    index += 1;
    while matches!(lower.get(index), Some(b' ' | b'\t')) {
        index += 1;
    }
    let start = index;
    while matches!(
        lower.get(index),
        Some(b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.')
    ) {
        index += 1;
    }
    if start == index {
        return None;
    }
    std::str::from_utf8(&line[comment + start..comment + index]).ok()
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
    if level > parts.len() {
        return Err("relative import goes beyond top-level package".to_string());
    }
    let base = parts[..parts.len() - (level - 1)].join(".");
    let name = raw_module.trim();
    if name.is_empty() {
        return Ok(base);
    }
    Ok(format!("{base}.{name}"))
}

#[cfg(test)]
mod tests {
    use super::{decode_python_source, resolve_relative_module_name};
    use std::path::Path;

    #[test]
    fn decodes_pep_263_latin_1_source() {
        let source = decode_python_source(
            b"# coding: latin-1\nVALUE = '\xe9'\n",
            Path::new("module.py"),
        )
        .expect("decode Latin-1 source");
        assert_eq!(source, "# coding: latin-1\nVALUE = 'é'\n");
    }

    #[test]
    fn rejects_conflicting_bom_and_encoding_cookie() {
        let error =
            decode_python_source(b"\xef\xbb\xbf# coding: latin-1\n", Path::new("module.py"))
                .expect_err("conflicting BOM must fail");
        assert!(error.contains("BOM conflicts"));
    }

    #[test]
    fn rejects_relative_import_beyond_top_level() {
        let error = resolve_relative_module_name("pkg.module", false, "other", 2)
            .expect_err("relative import beyond top level must fail");
        assert!(error.contains("beyond top-level"));
    }
}
