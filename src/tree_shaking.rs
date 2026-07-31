use std::path::Path;

use crate::analyzer::bundled_import_ranges;
use ruff_linter::linter::{ParseSource, lint_only};
use ruff_linter::registry::Rule;
use ruff_linter::settings::LinterSettings;
use ruff_linter::settings::flags;
use ruff_linter::settings::types::UnsafeFixes;
use ruff_linter::source_kind::SourceKind;
use ruff_python_ast::PySourceType;
use ruff_text_size::{Ranged, TextSize};

pub(crate) fn remove_unused_imports(source: &str) -> String {
    let path = Path::new("module.py");
    let settings = LinterSettings::for_rule(Rule::UnusedImport);
    let mut transformed = source.to_string();

    for _ in 0..100 {
        let source_kind = SourceKind::Python {
            code: transformed.clone(),
            is_stub: false,
        };
        let result = lint_only(
            path,
            None,
            &settings,
            flags::Noqa::Disabled,
            &source_kind,
            PySourceType::Python,
            ParseSource::None,
        );
        if result.has_invalid_syntax() {
            return source.to_string();
        }

        let protected = bundled_import_ranges(&transformed);
        let applicability = UnsafeFixes::Disabled.required_applicability();
        let mut fixes = result
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic
                    .range()
                    .is_none_or(|range| !protected.iter().any(|item| item.contains_range(range)))
            })
            .filter_map(|diagnostic| diagnostic.fix())
            .filter(|fix| fix.applies(applicability))
            .collect::<Vec<_>>();
        fixes.sort_by_key(|fix| fix.min_start());

        let mut output = String::with_capacity(transformed.len());
        let mut last_pos: Option<TextSize> = None;
        for fix in fixes {
            let Some(first) = fix.edits().first() else {
                continue;
            };
            if last_pos.is_some_and(|position| position >= first.start()) {
                continue;
            }
            for edit in fix.edits() {
                output.push_str(
                    &transformed[last_pos.unwrap_or_default().to_usize()..edit.start().to_usize()],
                );
                output.push_str(edit.content().unwrap_or_default());
                last_pos = Some(edit.end());
            }
        }
        let Some(last_pos) = last_pos else {
            return transformed;
        };
        output.push_str(&transformed[last_pos.to_usize()..]);
        transformed = output;
    }

    transformed
}
