use std::collections::{HashMap, HashSet};

use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{
    Alias, Expr, ExprAttribute, ExprCall, ExprName, ModModule, Number, Operator, Stmt,
    token::{TokenKind, Tokens},
};
use ruff_python_parser::{Parsed, parse_module};
use ruff_source_file::{LineIndex, OneIndexed};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::bundler::ModuleData;
use crate::codegen::module_package_name;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImportRequest {
    pub module_name: String,
    pub line: usize,
    pub is_relative: bool,
    pub relative_level: usize,
    pub must_resolve: bool,
    pub require_parent_packages: bool,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ModuleAnalysis {
    pub import_requests: Vec<ImportRequest>,
    pub multiline_string_continuation_lines: HashSet<usize>,
}

#[derive(Clone, Debug, Default)]
struct SkipDirectives {
    same_line: HashSet<usize>,
    next_line: HashSet<usize>,
}

#[derive(Clone, Debug, Default)]
struct EvalContext {
    module_name: String,
    package_name: String,
    file_path: String,
    string_constants: HashMap<String, String>,
    int_constants: HashMap<String, i32>,
}

#[derive(Clone, Debug, Default)]
struct AliasState {
    importlib_modules: HashSet<String>,
    import_module_fns: HashSet<String>,
    import_builtin_fns: HashSet<String>,
}

pub(crate) fn analyze_module(
    mod_data: &ModuleData,
    directive: &str,
) -> Result<ModuleAnalysis, String> {
    let source = String::from_utf8_lossy(&mod_data.source).to_string();
    let parsed = parse_module(&source)
        .map_err(|err| format!("parse {}: {err}", mod_data.file_path.display()))?;
    let module = parsed.syntax();
    let line_index = LineIndex::from_source_text(&source);

    let mut ctx = EvalContext {
        module_name: mod_data.name.clone(),
        package_name: module_package_name(&mod_data.name, mod_data.is_package),
        file_path: mod_data.file_path.display().to_string(),
        string_constants: HashMap::new(),
        int_constants: HashMap::new(),
    };
    collect_top_level_constants(&module.body, &mut ctx);
    let (skip_lines, multiline_lines) =
        collect_source_metadata(&source, &parsed, directive, &line_index);
    let aliases = collect_aliases(module);

    let mut collector = ImportCollector {
        requests: Vec::new(),
        walk_error: None,
        skip_lines,
        aliases,
        ctx,
        line_index,
    };
    for stmt in &module.body {
        collector.visit_stmt(stmt);
    }
    if let Some(err) = collector.walk_error {
        return Err(err);
    }

    Ok(ModuleAnalysis {
        import_requests: collector.requests,
        multiline_string_continuation_lines: multiline_lines,
    })
}

struct ImportCollector {
    requests: Vec<ImportRequest>,
    walk_error: Option<String>,
    skip_lines: SkipDirectives,
    aliases: AliasState,
    ctx: EvalContext,
    line_index: LineIndex,
}

impl ImportCollector {
    fn node_line(&self, range_start: TextSize) -> usize {
        self.line_index.line_index(range_start).to_zero_indexed()
    }
}

impl<'a> Visitor<'a> for ImportCollector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        if self.walk_error.is_some() {
            return;
        }
        match stmt {
            Stmt::Import(node) => {
                let line = self.node_line(node.range.start());
                if !is_skipped_import(line, &self.skip_lines) {
                    self.requests.extend(parse_import_statement(node, line));
                }
            }
            Stmt::ImportFrom(node) => {
                let line = self.node_line(node.range.start());
                if !is_skipped_import(line, &self.skip_lines) {
                    match parse_from_import_statement(node, line) {
                        Ok(reqs) => self.requests.extend(reqs),
                        Err(err) => {
                            self.walk_error =
                                Some(format!("{} line {}: {err}", self.ctx.file_path, line + 1));
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
        visitor::walk_stmt(self, stmt);
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        if self.walk_error.is_some() {
            return;
        }
        if let Expr::Call(call) = expr {
            let line = self.node_line(call.range.start());
            if !is_skipped_import(line, &self.skip_lines) {
                match parse_dynamic_import_call(call, line, &self.ctx, &self.aliases) {
                    Ok((reqs, handled)) => {
                        if handled {
                            self.requests.extend(reqs);
                        }
                    }
                    Err(err) => {
                        self.walk_error =
                            Some(format!("{} line {}: {err}", self.ctx.file_path, line + 1));
                        return;
                    }
                }
            }
        }
        visitor::walk_expr(self, expr);
    }
}

fn collect_source_metadata(
    source: &str,
    parsed: &Parsed<ModModule>,
    directive: &str,
    line_index: &LineIndex,
) -> (SkipDirectives, HashSet<usize>) {
    let mut lines = SkipDirectives {
        same_line: HashSet::new(),
        next_line: HashSet::new(),
    };
    let mut multiline = HashSet::new();
    let needle = directive.trim().to_ascii_lowercase();

    for token in parsed.tokens() {
        let token_range = token.range();
        let line = line_index.line_index(token_range.start()).to_zero_indexed();

        if token.kind() == TokenKind::Comment && !needle.is_empty() {
            let raw_comment = &source[token_range];
            let comment_body = raw_comment
                .strip_prefix('#')
                .unwrap_or(raw_comment)
                .trim()
                .to_ascii_lowercase();
            if comment_body.contains(&needle) {
                lines.same_line.insert(line);

                let line_start = line_index.line_start(OneIndexed::from_zero_indexed(line), source);
                let leading_range = TextRange::new(line_start, token_range.start());
                if source[leading_range].trim().is_empty() {
                    lines.next_line.insert(line);
                }
            }
        }

        if is_multiline_string_token(token, parsed.tokens(), line_index) {
            let end_line = line_index
                .line_index(token_range.end().saturating_sub(TextSize::new(1)))
                .to_zero_indexed();
            for continuation_line in (line + 1)..=end_line {
                multiline.insert(continuation_line);
            }
        }
    }

    (lines, multiline)
}

fn is_multiline_string_token(
    token: &ruff_python_ast::token::Token,
    tokens: &Tokens,
    line_index: &LineIndex,
) -> bool {
    if token.string_flags().is_none() {
        return false;
    }
    if !token.is_triple_quoted_string() {
        return false;
    }
    let range = token.range();
    let start_line = line_index.line_index(range.start()).to_zero_indexed();
    let end_line = line_index
        .line_index(range.end().saturating_sub(TextSize::new(1)))
        .to_zero_indexed();
    if end_line > start_line {
        return true;
    }

    // For split interpolated strings, a middle token can be single-line while the full string
    // spans multiple lines. Detect adjacent string parts that share the same logical string.
    let before = tokens.before(range.start());
    let after = tokens.after(range.end());
    before
        .last()
        .is_some_and(|prev| prev.string_flags().is_some() && prev.is_triple_quoted_string())
        || after
            .first()
            .is_some_and(|next| next.string_flags().is_some() && next.is_triple_quoted_string())
}

fn is_skipped_import(line: usize, skip_lines: &SkipDirectives) -> bool {
    skip_lines.same_line.contains(&line)
        || line
            .checked_sub(1)
            .is_some_and(|prev| skip_lines.next_line.contains(&prev))
}

fn parse_import_statement(node: &ruff_python_ast::StmtImport, line: usize) -> Vec<ImportRequest> {
    node.names
        .iter()
        .filter_map(parse_import_name)
        .map(|module_name| ImportRequest {
            module_name,
            line,
            is_relative: false,
            relative_level: 0,
            must_resolve: false,
            require_parent_packages: true,
        })
        .collect()
}

fn parse_from_import_statement(
    node: &ruff_python_ast::StmtImportFrom,
    line: usize,
) -> Result<Vec<ImportRequest>, String> {
    let base_module = node
        .module
        .as_ref()
        .map(|id| id.as_str().trim().to_string())
        .unwrap_or_default();
    let is_relative = node.level > 0;
    let relative_level = usize::try_from(node.level).unwrap_or(0);

    let mut requests = vec![ImportRequest {
        module_name: base_module.clone(),
        line,
        is_relative,
        relative_level,
        must_resolve: is_relative,
        require_parent_packages: true,
    }];

    for alias in &node.names {
        let Some(symbol) = parse_import_name(alias) else {
            continue;
        };
        if symbol == "*" {
            continue;
        }
        let submodule = if base_module.is_empty() {
            symbol
        } else {
            format!("{base_module}.{symbol}")
        };
        requests.push(ImportRequest {
            module_name: submodule,
            line,
            is_relative,
            relative_level,
            must_resolve: false,
            require_parent_packages: true,
        });
    }

    if requests.is_empty() {
        return Err("failed to parse from import statement".to_string());
    }
    Ok(requests)
}

fn parse_import_name(alias: &Alias) -> Option<String> {
    let module_name = alias.name.as_str().trim();
    if module_name.is_empty() {
        None
    } else {
        Some(module_name.to_string())
    }
}

fn parse_dynamic_import_call(
    node: &ExprCall,
    line: usize,
    ctx: &EvalContext,
    aliases: &AliasState,
) -> Result<(Vec<ImportRequest>, bool), String> {
    let Some(fn_kind) = identify_dynamic_import_function(&node.func, aliases) else {
        return Ok((Vec::new(), false));
    };
    let args = parse_call_arguments(node);

    match fn_kind {
        DynamicImportFunction::BuiltinImport => {
            let Some(name_node) = args
                .keyword
                .get("name")
                .copied()
                .or_else(|| args.positional.first().copied())
            else {
                return Err("__import__ requires module name argument".to_string());
            };
            let Some(name) = eval_string_expr(name_node, ctx) else {
                return Err("__import__ module name is not statically resolvable".to_string());
            };

            let level_node = args
                .keyword
                .get("level")
                .copied()
                .or_else(|| args.positional.get(4).copied());
            let level = if let Some(level_node) = level_node {
                let Some(value) = eval_int_expr(level_node, ctx) else {
                    return Err("__import__ level is not statically resolvable".to_string());
                };
                value
            } else {
                0
            };

            let (is_relative, relative_level, must_resolve) = if level > 0 {
                (true, level as usize, true)
            } else {
                (false, 0usize, false)
            };
            Ok((
                vec![ImportRequest {
                    module_name: name,
                    line,
                    is_relative,
                    relative_level,
                    must_resolve,
                    require_parent_packages: true,
                }],
                true,
            ))
        }
        DynamicImportFunction::ImportModule => {
            let Some(name_node) = args
                .keyword
                .get("name")
                .copied()
                .or_else(|| args.positional.first().copied())
            else {
                return Err("import_module requires module name argument".to_string());
            };
            let Some(raw_name) = eval_string_expr(name_node, ctx) else {
                return Err("import_module module name is not statically resolvable".to_string());
            };

            let mut module_name = raw_name.clone();
            let mut must_resolve = false;
            if raw_name.starts_with('.') {
                let package_node = args
                    .keyword
                    .get("package")
                    .copied()
                    .or_else(|| args.positional.get(1).copied());
                let Some(package_node) = package_node else {
                    return Err("relative import_module requires package argument".to_string());
                };
                let Some(package_name) = eval_string_expr(package_node, ctx) else {
                    return Err(
                        "import_module package argument is not statically resolvable".to_string(),
                    );
                };
                module_name = resolve_relative_name_with_package_arg(&package_name, &raw_name)?;
                must_resolve = true;
            }

            Ok((
                vec![ImportRequest {
                    module_name,
                    line,
                    is_relative: false,
                    relative_level: 0,
                    must_resolve,
                    require_parent_packages: true,
                }],
                true,
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DynamicImportFunction {
    BuiltinImport,
    ImportModule,
}

struct CallArguments<'a> {
    positional: Vec<&'a Expr>,
    keyword: HashMap<&'a str, &'a Expr>,
}

fn parse_call_arguments(call: &ExprCall) -> CallArguments<'_> {
    let positional = call.arguments.args.iter().collect::<Vec<_>>();
    let mut keyword = HashMap::new();
    for kw in &call.arguments.keywords {
        if let Some(arg) = kw.arg.as_ref() {
            keyword.insert(arg.as_str(), &kw.value);
        }
    }
    CallArguments {
        positional,
        keyword,
    }
}

fn identify_dynamic_import_function(
    fn_expr: &Expr,
    aliases: &AliasState,
) -> Option<DynamicImportFunction> {
    match fn_expr {
        Expr::Name(ExprName { id, .. }) => {
            let name = id.as_str();
            if name == "__import__" || aliases.import_builtin_fns.contains(name) {
                return Some(DynamicImportFunction::BuiltinImport);
            }
            if aliases.import_module_fns.contains(name) {
                return Some(DynamicImportFunction::ImportModule);
            }
            None
        }
        Expr::Attribute(ExprAttribute { value, attr, .. }) => {
            let attr_name = attr.as_str();
            if attr_name != "__import__" && attr_name != "import_module" {
                return None;
            }
            let Expr::Name(ExprName { id, .. }) = value.as_ref() else {
                return None;
            };
            let obj_name = id.as_str();
            if obj_name == "importlib" || aliases.importlib_modules.contains(obj_name) {
                if attr_name == "__import__" {
                    return Some(DynamicImportFunction::BuiltinImport);
                }
                return Some(DynamicImportFunction::ImportModule);
            }
            None
        }
        _ => None,
    }
}

fn collect_aliases(module: &ModModule) -> AliasState {
    struct Collector {
        state: AliasState,
    }

    impl<'a> Visitor<'a> for Collector {
        fn visit_stmt(&mut self, stmt: &'a Stmt) {
            match stmt {
                Stmt::Import(node) => {
                    for alias in &node.names {
                        let Some(module_name) = parse_import_name(alias) else {
                            continue;
                        };
                        let bound = imported_binding_name(alias);
                        if bound.is_empty() {
                            continue;
                        }
                        if module_name == "importlib" {
                            self.state.importlib_modules.insert(bound);
                        }
                    }
                }
                Stmt::ImportFrom(node) => {
                    let module_name = node
                        .module
                        .as_ref()
                        .map(|id| id.as_str().trim().to_string())
                        .unwrap_or_default();
                    for alias in &node.names {
                        let Some(name) = parse_import_name(alias) else {
                            continue;
                        };
                        let bound = imported_binding_name(alias);
                        if bound.is_empty() {
                            continue;
                        }
                        if module_name == "importlib" && name == "import_module" {
                            self.state.import_module_fns.insert(bound.clone());
                        }
                        if (module_name == "builtins" || module_name == "importlib")
                            && name == "__import__"
                        {
                            self.state.import_builtin_fns.insert(bound);
                        }
                    }
                }
                _ => {}
            }
            visitor::walk_stmt(self, stmt);
        }
    }

    let mut collector = Collector {
        state: AliasState::default(),
    };
    for stmt in &module.body {
        collector.visit_stmt(stmt);
    }
    collector.state
}

fn imported_binding_name(alias: &Alias) -> String {
    if let Some(as_name) = alias.asname.as_ref() {
        return as_name.as_str().to_string();
    }
    let name = alias.name.as_str().trim();
    if name.is_empty() {
        return String::new();
    }
    name.split('.').next().unwrap_or_default().to_string()
}

fn collect_top_level_constants(body: &[Stmt], ctx: &mut EvalContext) {
    for stmt in body {
        let Stmt::Assign(assign) = stmt else {
            continue;
        };
        for target in &assign.targets {
            let Expr::Name(name) = target else {
                continue;
            };
            let id = name.id.as_str().to_string();
            if let Some(value) = eval_string_expr(&assign.value, ctx) {
                ctx.string_constants.insert(id.clone(), value);
            } else {
                ctx.string_constants.remove(&id);
            }
            if let Some(value) = eval_int_expr(&assign.value, ctx) {
                ctx.int_constants.insert(id.clone(), value);
            } else {
                ctx.int_constants.remove(&id);
            }
        }
    }
}

fn eval_string_expr(node: &Expr, ctx: &EvalContext) -> Option<String> {
    match node {
        Expr::StringLiteral(expr) => Some(expr.value.to_str().to_string()),
        Expr::BinOp(expr) => {
            if expr.op != Operator::Add {
                return None;
            }
            let left = eval_string_expr(&expr.left, ctx)?;
            let right = eval_string_expr(&expr.right, ctx)?;
            Some(format!("{left}{right}"))
        }
        Expr::Name(expr) => {
            let name = expr.id.as_str();
            if let Some(value) = ctx.string_constants.get(name) {
                return Some(value.clone());
            }
            match name {
                "__name__" => Some(ctx.module_name.clone()),
                "__package__" => Some(ctx.package_name.clone()),
                "__file__" => Some(ctx.file_path.clone()),
                _ => None,
            }
        }
        Expr::Attribute(expr) => {
            let Expr::Name(base) = expr.value.as_ref() else {
                return None;
            };
            if base.id.as_str() != "__spec__" {
                return None;
            }
            match expr.attr.as_str() {
                "name" => Some(ctx.module_name.clone()),
                "parent" => Some(ctx.package_name.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

fn eval_int_expr(node: &Expr, ctx: &EvalContext) -> Option<i32> {
    match node {
        Expr::NumberLiteral(expr) => match &expr.value {
            Number::Int(value) => value.as_i32(),
            _ => None,
        },
        Expr::Name(expr) => ctx.int_constants.get(expr.id.as_str()).copied(),
        _ => None,
    }
}

fn resolve_relative_name_with_package_arg(
    package_name: &str,
    raw_name: &str,
) -> Result<String, String> {
    let level = raw_name.chars().take_while(|ch| *ch == '.').count();
    if level == 0 {
        return Ok(raw_name.to_string());
    }
    if package_name.is_empty() {
        return Err(format!(
            "relative import {:?} requires non-empty package context",
            raw_name
        ));
    }
    let tail = &raw_name[level..];
    let parts = package_name.split('.').collect::<Vec<_>>();
    if level - 1 > parts.len() {
        return Err(format!(
            "relative import {:?} goes beyond top-level package {:?}",
            raw_name, package_name
        ));
    }
    let base = parts[..parts.len() - (level - 1)].join(".");
    if tail.is_empty() {
        return Ok(base);
    }
    if base.is_empty() {
        return Ok(tail.to_string());
    }
    Ok(format!("{base}.{tail}"))
}
