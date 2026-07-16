use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use crate::analyzer::{ModuleAnalysis, analyze_module};
use crate::codegen::generate_bundle_code;
use crate::licenses::collect_license_comments;
use crate::module_graph::{ensure_parent_packages, module_name_from_path};
use crate::resolver::{ModuleResolver, read_module_source};
use crate::sys_paths::discover_sys_paths;
use crate::tree_shaking::remove_unused_imports;

const DEFAULT_IGNORE_DIRECTIVE: &str = "no-bundle";
const DEFAULT_MAX_IMPORTED_MODULES: usize = 2048;

/// Configuration for [`bundle_file`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleOptions {
    /// Top-level package names that should stay external (not bundled).
    pub external: Vec<String>,
    /// Comment marker used to skip bundling on import lines.
    ///
    /// Defaults to `"no-bundle"`.
    pub ignore_comment_literal: String,
    /// Maximum number of imported modules allowed during graph expansion.
    ///
    /// Defaults to 2048.
    pub max_imported_modules: usize,
    /// Python interpreters used to discover `sys.path` directories.
    ///
    /// Each interpreter is invoked to print its `sys.path` entries.
    /// The resulting directories are searched in this order when a
    /// module is not found under the project root.
    ///
    /// When empty, no `sys.path` discovery is performed.
    pub interpreter: Vec<String>,
    /// Whether to remove unused imports from the bundled output.
    ///
    /// Defaults to `true`.
    pub tree_shaking: bool,
}

impl Default for BundleOptions {
    fn default() -> Self {
        Self {
            external: Vec::new(),
            ignore_comment_literal: DEFAULT_IGNORE_DIRECTIVE.to_string(),
            max_imported_modules: DEFAULT_MAX_IMPORTED_MODULES,
            interpreter: Vec::new(),
            tree_shaking: true,
        }
    }
}

/// Metadata for a module included in the bundle graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundledModule {
    /// Dotted module name, for example `pkg.sub.module`.
    pub name: String,
    /// Original file path used to load this module.
    pub file_path: String,
    /// Whether the module represents a package (`__init__.py`).
    pub is_package: bool,
    /// Whether the package node was synthesized to preserve package hierarchy.
    pub synthetic: bool,
}

/// Result returned by [`bundle_file`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleResult {
    /// Generated bundled Python code.
    pub code: String,
    /// Absolute path of the entry file.
    pub entry_file: String,
    /// Dotted module name of the entry file.
    pub entry_module: String,
    /// All modules included in the resolved dependency graph.
    pub bundled_module_list: Vec<BundledModule>,
}

#[derive(Clone, Debug)]
pub(crate) struct ModuleData {
    pub name: String,
    pub file_path: PathBuf,
    pub is_package: bool,
    pub synthetic: bool,
    pub source: Vec<u8>,
    pub analysis: Option<ModuleAnalysis>,
}

#[derive(Clone, Debug)]
pub(crate) struct ImportedModuleBudget {
    pub max_imported_modules: usize,
    pub imported_count: usize,
}

impl ImportedModuleBudget {
    pub(crate) fn track(&mut self, module_name: &str) -> Result<(), String> {
        if self.imported_count >= self.max_imported_modules {
            return Err(format!(
                "imported module limit of {} exceeded while resolving {:?}",
                self.max_imported_modules, module_name
            ));
        }
        self.imported_count += 1;
        Ok(())
    }
}

/// Bundles an entry Python file and its local dependencies into one executable script.
///
/// This function:
/// - parses the entry module and discovered imports,
/// - resolves modules relative to the entry file's parent directory,
/// - preserves package semantics (including synthetic parent packages when needed),
/// - and returns generated Python code plus module graph metadata.
///
/// Imports can be controlled with [`BundleOptions`]:
/// - `external`: keeps selected top-level packages as runtime imports instead of bundling.
/// - `ignore_comment_literal`: skips imports annotated with the marker (default: `no-bundle`).
/// - `max_imported_modules`: protects against runaway dependency expansion.
///
/// # Errors
///
/// Returns an error when:
/// - `entry_file` is empty, not a `.py` file, or points to a directory,
/// - module parsing/resolution fails for required imports,
/// - the module expansion exceeds `max_imported_modules`.
///
/// # Examples
///
/// ```no_run
/// use pybundler::{bundle_file, BundleOptions};
///
/// let result = bundle_file(
///     "src/main.py",
///     BundleOptions {
///         external: vec!["numpy".to_string()],
///         ..BundleOptions::default()
///     },
/// )?;
///
/// assert!(!result.code.is_empty());
/// # Ok::<(), String>(())
/// ```
pub fn bundle_file(entry_file: &str, opts: BundleOptions) -> Result<BundleResult, String> {
    if entry_file.trim().is_empty() {
        return Err("entry file is required".to_string());
    }

    let abs_entry = canonical_abs(Path::new(entry_file), "resolve entry file path")?;
    let entry_meta = fs::metadata(&abs_entry).map_err(|err| format!("stat entry file: {err}"))?;
    if entry_meta.is_dir() {
        return Err(format!(
            "entry file must be a Python file, got directory: {}",
            abs_entry.display()
        ));
    }
    if !has_py_extension(&abs_entry) {
        return Err(format!(
            "entry file must end with .py: {}",
            abs_entry.display()
        ));
    }

    let project_root = abs_entry
        .parent()
        .ok_or_else(|| "entry file must have parent directory".to_string())?
        .to_path_buf();

    let directive = opts.ignore_comment_literal.trim().to_string();
    let external = normalize_external_prefixes(&opts.external);
    let max_imported_modules = opts.max_imported_modules;
    let tree_shaking_enabled = opts.tree_shaking;
    let mut import_budget = ImportedModuleBudget {
        max_imported_modules,
        imported_count: 0,
    };

    let (entry_module, entry_is_package) = module_name_from_path(&project_root, &abs_entry)?;

    let mut search_roots = vec![project_root.clone()];
    search_roots.extend(discover_sys_paths(&opts.interpreter));
    let resolver = ModuleResolver::new(search_roots.clone());
    let mut module_map: HashMap<String, ModuleData> = HashMap::new();
    let mut analysis_cache: HashMap<PathBuf, (ModuleAnalysis, Vec<u8>)> = HashMap::new();
    let mut queue = VecDeque::from([entry_module.clone()]);
    module_map.insert(
        entry_module.clone(),
        ModuleData {
            name: entry_module.clone(),
            file_path: abs_entry.clone(),
            is_package: entry_is_package,
            synthetic: false,
            source: Vec::new(),
            analysis: None,
        },
    );

    while let Some(current_name) = queue.pop_front() {
        let Some(current_snapshot) = module_map.get(&current_name).cloned() else {
            return Err(format!(
                "internal error: missing queued module {:?}",
                current_name
            ));
        };
        if current_snapshot.synthetic {
            continue;
        }

        if !analysis_cache.contains_key(&current_snapshot.file_path) {
            let source = read_module_source(&current_snapshot.file_path).map_err(|err| {
                format!(
                    "read module {:?} ({}): {err}",
                    current_snapshot.name,
                    current_snapshot.file_path.display()
                )
            })?;
            let mut analyzed = current_snapshot.clone();
            if tree_shaking_enabled {
                let source_str = String::from_utf8_lossy(&source).to_string();
                analyzed.source = remove_unused_imports(&source_str).into_bytes();
            } else {
                analyzed.source = source;
            }
            analyzed.analysis = Some(analyze_module(&analyzed, &directive)?);
            analysis_cache.insert(
                current_snapshot.file_path.clone(),
                (
                    analyzed
                        .analysis
                        .clone()
                        .ok_or_else(|| "internal error: missing analysis".to_string())?,
                    analyzed.source.clone(),
                ),
            );
            if let Some(current) = module_map.get_mut(&current_name) {
                current.source = analyzed.source;
                current.analysis = analyzed.analysis;
            }
        } else {
            if let Some(current) = module_map.get_mut(&current_name) {
                let (analysis, shaken) = analysis_cache
                    .get(&current_snapshot.file_path)
                    .ok_or_else(|| "internal error: missing cache entry".to_string())?;
                current.source.clone_from(shaken);
                current.analysis = Some(analysis.clone());
            }
        }

        let current_module = module_map
            .get(&current_name)
            .cloned()
            .ok_or_else(|| format!("internal error: missing module {:?}", current_name))?;
        let analysis = current_module
            .analysis
            .ok_or_else(|| format!("internal error: analysis not found for {:?}", current_name))?;

        for req in &analysis.import_requests {
            let mut target_name = req.module_name.clone();
            if req.is_relative {
                target_name = crate::resolver::resolve_relative_module_name(
                    &current_module.name,
                    current_module.is_package,
                    &req.module_name,
                    req.relative_level,
                )
                .map_err(|err| {
                    format!(
                        "resolve relative import in {} at line {}: {err}",
                        current_module.file_path.display(),
                        req.line + 1
                    )
                })?;
            }
            if should_preserve_external_import(&target_name, &external) {
                continue;
            }

            let resolved = resolver.resolve_module(&target_name).map_err(|err| {
                format!(
                    "resolve import {:?} in {} at line {}: {err}",
                    target_name,
                    current_module.file_path.display(),
                    req.line + 1
                )
            })?;
            let Some(resolved) = resolved else {
                if req.must_resolve {
                    return Err(format!(
                        "failed to resolve import {:?} in {} at line {}",
                        target_name,
                        current_module.file_path.display(),
                        req.line + 1
                    ));
                }
                continue;
            };

            let mut changed = false;
            if !module_map.contains_key(&resolved.name) {
                import_budget.track(&resolved.name)?;
                module_map.insert(
                    resolved.name.clone(),
                    ModuleData {
                        name: resolved.name.clone(),
                        file_path: resolved.file_path.clone(),
                        is_package: resolved.is_package,
                        synthetic: false,
                        source: Vec::new(),
                        analysis: None,
                    },
                );
                queue.push_back(resolved.name.clone());
                changed = true;
            }

            if changed || req.require_parent_packages {
                ensure_parent_packages(
                    &target_name,
                    &resolver,
                    &mut module_map,
                    &mut queue,
                    &mut import_budget,
                )?;
            }
        }
    }

    // Build sorted module list to match codegen order
    let sorted_modules = {
        let mut names: Vec<String> = module_map.keys().cloned().collect();
        names.sort();
        names
    };

    // Collect license information from dist-info directories
    let license_comments = collect_license_comments(&search_roots, &module_map, &sorted_modules);

    // Build map of module name -> formatted license header strings
    let mut license_headers: HashMap<String, Vec<String>> = HashMap::new();
    for comment in &license_comments {
        let escaped = comment.text.replace("\"\"\"", "\"\"\\\"");
        let header = format!(
            "\"\"\"\n===== {} {} =====\n\n{}\n\"\"\"\n\n",
            comment.package_name,
            comment.version,
            escaped.trim_matches(|c| c == '\n' || c == '\r')
        );
        license_headers
            .entry(comment.target_module.clone())
            .or_default()
            .push(header);
    }

    let entry_module_data = module_map
        .get(&entry_module)
        .cloned()
        .ok_or_else(|| format!("internal error: entry module {:?} missing", entry_module))?;

    let code = generate_bundle_code(&entry_module_data, &module_map, &license_headers);
    let mut module_list = module_map
        .values()
        .map(|mod_data| BundledModule {
            name: mod_data.name.clone(),
            file_path: mod_data.file_path.display().to_string(),
            is_package: mod_data.is_package,
            synthetic: mod_data.synthetic,
        })
        .collect::<Vec<_>>();
    module_list.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(BundleResult {
        code,
        entry_file: abs_entry.display().to_string(),
        entry_module,
        bundled_module_list: module_list,
    })
}

fn canonical_abs(path: &Path, context: &str) -> Result<PathBuf, String> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| format!("{context}: {err}"))?
            .join(path)
    };
    use path_clean::PathClean;

    Ok(abs.clean())
}

fn has_py_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("py"))
}

fn normalize_external_prefixes(prefixes: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for raw_prefix in prefixes {
        let mut prefix = raw_prefix.trim().replace(['\\', '/'], ".");
        prefix = prefix.trim_matches('.').to_string();
        if prefix.is_empty() {
            continue;
        }
        if let Some(idx) = prefix.find('.') {
            prefix = prefix[..idx].to_string();
        }
        if !prefix.is_empty() {
            out.insert(prefix);
        }
    }
    out
}

fn should_preserve_external_import(module_name: &str, external: &HashSet<String>) -> bool {
    if external.is_empty() {
        return false;
    }
    let first = module_name.split('.').next().unwrap_or_default();
    external.contains(first)
}
