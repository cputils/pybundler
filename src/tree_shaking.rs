use std::path::Path;

use ruff_linter::linter::lint_fix;
use ruff_linter::registry::Rule;
use ruff_linter::settings::LinterSettings;
use ruff_linter::settings::flags;
use ruff_linter::settings::types::UnsafeFixes;
use ruff_linter::source_kind::SourceKind;
use ruff_python_ast::PySourceType;

pub(crate) fn remove_unused_imports(source: &str) -> String {
    let path = Path::new("module.py");
    let source_kind = SourceKind::Python {
        code: source.to_string(),
        is_stub: false,
    };
    let settings = LinterSettings::for_rule(Rule::UnusedImport);

    let result = match lint_fix(
        path,
        None,
        flags::Noqa::Enabled,
        UnsafeFixes::Disabled,
        &settings,
        &source_kind,
        PySourceType::Python,
    ) {
        Ok(r) => r,
        Err(_) => return source.to_string(),
    };

    result.transformed.source_code().to_string()
}
