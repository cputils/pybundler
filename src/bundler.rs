use std::collections::{HashMap, HashSet, VecDeque, hash_map::Entry};
use std::fs;
use std::path::{Path, PathBuf};

use crate::analyzer::{ImportRequest, ModuleAnalysis, analyze_module};
use crate::codegen::generate_bundle_code;
use crate::licenses::collect_license_comments;
use crate::module_graph::{ensure_parent_packages, module_name_from_path};
use crate::resolver::{ModuleResolver, ResolvedModule, read_module_source};
use crate::sys_paths::discover_sys_paths;
use crate::tree_shaking::remove_unused_imports;
use ruff_python_formatter::{PyFormatOptions, format_module_source};

const DEFAULT_MAX_IMPORTED_MODULES: usize = 2048;

/// Configuration for [`bundle_file`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleOptions {
    /// Top-level package names that should stay external (not bundled).
    pub external: Vec<String>,
    /// Maximum number of imported modules allowed during graph expansion.
    ///
    /// Defaults to 2048.
    pub max_imported_modules: usize,
    /// Python interpreters used to discover `sys.path` entries.
    ///
    /// Each interpreter is invoked to print its `sys.path` entries.
    /// The resulting directories and ZIP paths are searched in this order when a
    /// module is not found under the project root. Imports resolved from
    /// these directories require a `# bundle` directive by default; set
    /// [`BundleOptions::require_bundle_directive`] to `false` to disable this
    /// requirement. Imports discovered recursively from an admitted module do
    /// not require a directive.
    ///
    /// When empty, no `sys.path` discovery is performed.
    pub interpreter: Vec<String>,
    /// Whether imports resolved through an interpreter's `sys.path` require a
    /// `# bundle` directive.
    ///
    /// Defaults to `true`.
    pub require_bundle_directive: bool,
    /// Whether to remove unused imports from the bundled output.
    ///
    /// Imports marked with `# bundle` are preserved.
    ///
    /// Defaults to `true`.
    pub tree_shaking: bool,
    /// Whether to format the bundled output with Ruff.
    ///
    /// Defaults to `false`.
    pub format: bool,
}

impl Default for BundleOptions {
    fn default() -> Self {
        Self {
            external: Vec::new(),
            max_imported_modules: DEFAULT_MAX_IMPORTED_MODULES,
            interpreter: Vec::new(),
            require_bundle_directive: true,
            tree_shaking: true,
            format: false,
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
    /// Canonical absolute path of the entry file.
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
    pub allow_sys_path_imports: bool,
    pub source: Vec<u8>,
    pub analysis: Option<ModuleAnalysis>,
}

#[derive(Clone, Debug)]
pub(crate) struct ImportedModuleBudget {
    pub max_imported_modules: usize,
    pub imported_count: usize,
}

type AnalysisCache = HashMap<PathBuf, (ModuleAnalysis, Vec<u8>)>;

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
/// - `max_imported_modules`: protects against runaway dependency expansion.
///
/// Imports resolved through an interpreter's `sys.path` require a `# bundle`
/// directive at the project boundary by default. Set
/// [`BundleOptions::require_bundle_directive`] to `false` to include them
/// without the directive. Transitive imports of an admitted module are bundled
/// recursively without requiring additional directives.
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
    let (abs_entry, project_root) = resolve_entry_file(entry_file)?;

    let external = normalize_external_prefixes(&opts.external);
    let max_imported_modules = opts.max_imported_modules;
    let tree_shaking_enabled = opts.tree_shaking;
    let allow_sys_path_imports = !opts.require_bundle_directive;
    let mut import_budget = ImportedModuleBudget {
        max_imported_modules,
        imported_count: 0,
    };

    let (entry_module, entry_is_package) = module_name_from_path(&project_root, &abs_entry)?;

    let sys_path_roots = discover_sys_paths(&opts.interpreter);
    let mut search_roots = vec![project_root.clone()];
    search_roots.extend(sys_path_roots.iter().cloned());
    let resolver = ModuleResolver::new(project_root.clone(), sys_path_roots);
    let mut module_map: HashMap<String, ModuleData> = HashMap::new();
    let mut analysis_cache = AnalysisCache::new();
    let mut pending_all_expansions: HashMap<String, bool> = HashMap::new();
    let mut completed_all_expansions: HashMap<String, bool> = HashMap::new();
    let mut force_runtime = false;
    let mut queue = VecDeque::from([entry_module.clone()]);
    module_map.insert(
        entry_module.clone(),
        ModuleData {
            name: entry_module.clone(),
            file_path: abs_entry.clone(),
            is_package: entry_is_package,
            synthetic: false,
            allow_sys_path_imports,
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

        let (analysis, source) =
            analyze_cached_module(&current_snapshot, tree_shaking_enabled, &mut analysis_cache)?;
        if let Some(current) = module_map.get_mut(&current_name) {
            current.source = source;
            current.analysis = Some(analysis);
        }

        let current_module = module_map
            .get(&current_name)
            .cloned()
            .ok_or_else(|| format!("internal error: missing module {:?}", current_name))?;
        let analysis = current_module
            .analysis
            .ok_or_else(|| format!("internal error: analysis not found for {:?}", current_name))?;

        let expansion_force = pending_all_expansions.remove(&current_name);
        if let Some(force_bundle) = expansion_force {
            completed_all_expansions
                .entry(current_name.clone())
                .and_modify(|completed| *completed |= force_bundle)
                .or_insert(force_bundle);
        }
        let mut import_requests = analysis.import_requests;
        if let Some(force_bundle) = expansion_force {
            import_requests.extend(
                analysis
                    .all_names
                    .into_iter()
                    .filter(|name| !name.is_empty() && name != "*")
                    .map(|name| ImportRequest {
                        module_name: format!("{}.{}", current_module.name, name),
                        line: 0,
                        is_relative: false,
                        relative_level: 0,
                        must_resolve: false,
                        require_parent_packages: true,
                        force_bundle,
                        expand_all: false,
                    }),
            );
        }

        for req in &import_requests {
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
            if !req.force_bundle && should_preserve_external_import(&target_name, &external) {
                continue;
            }
            if is_guaranteed_builtin(&target_name) {
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
                if req.require_parent_packages {
                    ensure_parent_packages(
                        &target_name,
                        &resolver,
                        &mut module_map,
                        &mut queue,
                        &mut import_budget,
                        current_module.allow_sys_path_imports || req.force_bundle,
                    )?;
                }
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
            if resolved.from_sys_path && !req.force_bundle && !current_module.allow_sys_path_imports
            {
                continue;
            }
            force_runtime |= resolved.name == entry_module;
            if req.expand_all && resolved.is_package && !resolved.synthetic {
                let completed_force = completed_all_expansions
                    .get(&resolved.name)
                    .copied()
                    .unwrap_or(false);
                let pending_force = pending_all_expansions
                    .get(&resolved.name)
                    .copied()
                    .unwrap_or(false);
                if (req.force_bundle && !completed_force && !pending_force)
                    || (!req.force_bundle
                        && !completed_all_expansions.contains_key(&resolved.name)
                        && !pending_all_expansions.contains_key(&resolved.name))
                {
                    pending_all_expansions.insert(resolved.name.clone(), req.force_bundle);
                    queue.push_back(resolved.name.clone());
                }
            }

            let changed = include_resolved_module(
                &mut module_map,
                &mut queue,
                &mut import_budget,
                &resolved,
                current_module.allow_sys_path_imports,
            )?;

            if changed || req.require_parent_packages {
                ensure_parent_packages(
                    &target_name,
                    &resolver,
                    &mut module_map,
                    &mut queue,
                    &mut import_budget,
                    current_module.allow_sys_path_imports || resolved.from_sys_path,
                )?;
            }
        }
    }

    if opts.format {
        for module in module_map.values_mut().filter(|module| !module.synthetic) {
            let source = String::from_utf8_lossy(&module.source);
            if let Ok(printed) = format_module_source(&source, PyFormatOptions::default()) {
                module.source = printed.into_code().into_bytes();
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

    let license_headers = format_license_headers(&license_comments);

    let entry_module_data = module_map
        .get(&entry_module)
        .cloned()
        .ok_or_else(|| format!("internal error: entry module {:?} missing", entry_module))?;

    let mut code = generate_bundle_code(
        &entry_module_data,
        &module_map,
        &license_headers,
        force_runtime,
    );
    if opts.format {
        code = format_module_source(&code, PyFormatOptions::default())
            .map(|printed| printed.into_code())
            .unwrap_or(code);
    }
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

fn resolve_entry_file(entry_file: &str) -> Result<(PathBuf, PathBuf), String> {
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
    Ok((abs_entry, project_root))
}

fn analyze_cached_module(
    module: &ModuleData,
    tree_shaking: bool,
    cache: &mut AnalysisCache,
) -> Result<(ModuleAnalysis, Vec<u8>), String> {
    if let Some(cached) = cache.get(&module.file_path) {
        return Ok(cached.clone());
    }

    let source = read_module_source(&module.file_path).map_err(|err| {
        format!(
            "read module {:?} ({}): {err}",
            module.name,
            module.file_path.display()
        )
    })?;
    let source = if tree_shaking {
        remove_unused_imports(&String::from_utf8_lossy(&source)).into_bytes()
    } else {
        source
    };
    let mut analyzed = module.clone();
    analyzed.source.clone_from(&source);
    let cached = (analyze_module(&analyzed)?, source);
    cache.insert(module.file_path.clone(), cached.clone());
    Ok(cached)
}

fn include_resolved_module(
    module_map: &mut HashMap<String, ModuleData>,
    queue: &mut VecDeque<String>,
    import_budget: &mut ImportedModuleBudget,
    resolved: &ResolvedModule,
    allow_sys_path_imports: bool,
) -> Result<bool, String> {
    match module_map.entry(resolved.name.clone()) {
        Entry::Vacant(entry) => {
            import_budget.track(&resolved.name)?;
            entry.insert(ModuleData {
                name: resolved.name.clone(),
                file_path: resolved.file_path.clone(),
                is_package: resolved.is_package,
                synthetic: resolved.synthetic,
                allow_sys_path_imports: allow_sys_path_imports || resolved.from_sys_path,
                source: Vec::new(),
                analysis: None,
            });
            if !resolved.synthetic {
                queue.push_back(resolved.name.clone());
            }
            Ok(true)
        }
        Entry::Occupied(mut entry)
            if allow_sys_path_imports && !entry.get().allow_sys_path_imports =>
        {
            entry.get_mut().allow_sys_path_imports = true;
            queue.push_back(resolved.name.clone());
            Ok(true)
        }
        Entry::Occupied(_) => Ok(false),
    }
}

fn format_license_headers(
    comments: &[crate::licenses::LicenseComment],
) -> HashMap<String, Vec<String>> {
    let mut headers: HashMap<String, Vec<String>> = HashMap::new();
    for comment in comments {
        let mut header = format!(
            "# ===== {} {} =====\n#\n",
            comment.package_name, comment.version
        );
        for line in comment.text.trim_matches(['\n', '\r']).lines() {
            header.push_str("# ");
            header.push_str(line);
            header.push('\n');
        }
        header.push('\n');
        headers
            .entry(comment.target_module.clone())
            .or_default()
            .push(header);
    }
    headers
}

fn canonical_abs(path: &Path, context: &str) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|err| format!("{context} {}: {err}", path.display()))
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

fn is_guaranteed_builtin(module_name: &str) -> bool {
    matches!(
        module_name.split('.').next().unwrap_or_default(),
        "builtins" | "sys" | "_imp"
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::canonical_abs;
    use std::os::unix::fs::symlink;

    #[test]
    fn canonicalizes_parent_components_after_symlinks() {
        let root = std::env::temp_dir().join(format!("pybundler-canonical-{}", std::process::id()));
        let real = root.join("real");
        let apparent = root.join("apparent");
        std::fs::create_dir_all(real.join("sub")).expect("create real directory");
        std::fs::create_dir_all(&apparent).expect("create apparent directory");
        std::fs::write(real.join("main.py"), b"pass\n").expect("write entry module");
        symlink(real.join("sub"), apparent.join("link")).expect("create directory symlink");

        let resolved = canonical_abs(&apparent.join("link/../main.py"), "resolve")
            .expect("resolve symlinked path");

        std::fs::remove_dir_all(&root).expect("remove test tree");
        assert_eq!(resolved, real.join("main.py"));
    }
}
