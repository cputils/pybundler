use std::collections::{HashMap, HashSet};

use crate::bundler::ModuleData;

pub(crate) fn generate_bundle_code(
    entry: &ModuleData,
    modules: &HashMap<String, ModuleData>,
    license_headers: &HashMap<String, Vec<String>>,
    force_runtime: bool,
) -> String {
    let mut names = modules.keys().cloned().collect::<Vec<_>>();
    names.sort();

    if names.len() == 1 && !force_runtime {
        let mut out = String::new();
        append_module_license_comments(&mut out, entry, license_headers);
        out.push_str(&normalize_python_newlines(&String::from_utf8_lossy(
            &entry.source,
        )));
        return out;
    }

    let mut out = String::new();
    let mut future_features = modules
        .values()
        .filter_map(|module| module.analysis.as_ref())
        .flat_map(|analysis| analysis.future_features.iter().cloned())
        .collect::<Vec<_>>();
    future_features.sort();
    future_features.dedup();
    if !future_features.is_empty() {
        out.push_str("from __future__ import ");
        out.push_str(&future_features.join(", "));
        out.push_str("\n\n");
    }

    append_module_license_comments(&mut out, entry, license_headers);
    out.push_str(&format!(
        "if __name__ == {:?} or (__name__ == \"__main__\" and globals().get(\"_COLLECTING\")):\n",
        entry.name
    ));
    append_module_body(&mut out, entry);
    out.push('\n');

    for name in &names {
        if *name == entry.name {
            continue;
        }
        let Some(module) = modules.get(name) else {
            continue;
        };
        append_module_license_comments(&mut out, module, license_headers);
        out.push_str(&format!("if __name__ == {name:?}:\n"));
        append_module_body(&mut out, module);
        out.push('\n');
    }

    out.push_str("if not globals().get(\"_COLLECTING\"):\n");
    out.push_str("\t# The following runtime code is part of pybundler.\n");
    out.push_str("\t# https://github.com/cputils/pybundler\n");
    out.push_str("\t# SPDX-License-Identifier: CC0-1.0\n");
    out.push('\n');
    out.push_str("\tdef _setup():\n");
    out.push_str("\t\timport importlib.abc\n");
    out.push_str("\t\timport importlib.machinery\n");
    out.push_str("\t\timport importlib.util\n");
    out.push_str("\t\timport sys\n");
    out.push('\n');

    out.push_str("\t\tmodules_info = {\n");
    for name in &names {
        let Some(module) = modules.get(name) else {
            continue;
        };
        let origin = if module.synthetic {
            "None".to_string()
        } else {
            format!("{:?}", module_origin(module))
        };
        let is_package = if module.is_package { "True" } else { "False" };
        let synthetic = if module.synthetic { "True" } else { "False" };
        out.push_str(&format!(
            "\t\t\t{name:?}: ({origin}, {is_package}, {synthetic}),\n"
        ));
    }
    out.push_str("\t\t}\n");
    out.push('\n');
    out.push_str("\t\tframe = sys._getframe().f_back\n");
    out.push_str("\t\tassert frame is not None\n");
    out.push_str("\t\tcode = frame.f_code\n");
    out.push('\n');

    out.push_str("\t\tclass Loader(importlib.abc.Loader):\n");
    out.push_str("\t\t\tdef exec_module(self, module):\n");
    out.push_str("\t\t\t\tsetattr(module, \"_COLLECTING\", True)\n");
    out.push_str("\t\t\t\texec(code, module.__dict__)\n");
    out.push('\n');
    out.push_str("\t\t\tdef get_filename(self, fullname):\n");
    out.push_str("\t\t\t\treturn modules_info[fullname][0]\n");
    out.push('\n');
    out.push_str("\t\t\tdef get_code(self, fullname):\n");
    out.push_str("\t\t\t\treturn code\n");
    out.push('\n');
    out.push_str("\t\t\tdef is_package(self, fullname):\n");
    out.push_str("\t\t\t\treturn modules_info[fullname][1]\n");
    out.push('\n');

    out.push_str("\t\tclass Finder(importlib.abc.MetaPathFinder):\n");
    out.push_str("\t\t\tdef find_spec(self, fullname, path=None, target=None):\n");
    out.push_str("\t\t\t\tinfo = modules_info.get(fullname)\n");
    out.push_str("\t\t\t\tif info is None:\n");
    out.push_str("\t\t\t\t\treturn None\n");
    out.push_str("\t\t\t\tif path is not None:\n");
    out.push_str("\t\t\t\t\tpackage_path = fullname.rpartition(\".\")[0].replace(\".\", \"/\")\n");
    out.push_str("\t\t\t\t\tif package_path not in path:\n");
    out.push_str("\t\t\t\t\t\treturn None\n");
    out.push_str("\t\t\t\tif info[2]:\n");
    out.push_str(
        "\t\t\t\t\tspec = importlib.util.spec_from_loader(fullname, None, is_package=True)\n",
    );
    out.push_str("\t\t\t\t\tspec.submodule_search_locations.append(\n");
    out.push_str("\t\t\t\t\t\tfullname.replace(\".\", \"/\")\n");
    out.push_str("\t\t\t\t\t)\n");
    out.push_str("\t\t\t\t\treturn spec\n");
    out.push_str(
        "\t\t\t\treturn importlib.util.spec_from_loader(fullname, loader, is_package=info[1])\n",
    );
    out.push('\n');

    out.push_str("\t\tloader = Loader()\n");
    out.push_str("\t\tfinder_index = next(\n");
    out.push_str("\t\t\t(\n");
    out.push_str("\t\t\t\tindex\n");
    out.push_str("\t\t\t\tfor index, finder in enumerate(sys.meta_path)\n");
    out.push_str("\t\t\t\tif finder is importlib.machinery.PathFinder\n");
    out.push_str("\t\t\t),\n");
    out.push_str("\t\t\tlen(sys.meta_path),\n");
    out.push_str("\t\t)\n");
    out.push_str("\t\tsys.meta_path.insert(finder_index, Finder())\n");
    out.push('\n');

    let entry_package = module_package_name(&entry.name, entry.is_package);
    let entry_origin = module_origin(entry);
    let entry_is_package = if entry.is_package { "True" } else { "False" };
    out.push_str("\t\tif __name__ == \"__main__\":\n");
    out.push_str("\t\t\tmain_mod = sys.modules[\"__main__\"]\n");
    out.push_str(&format!("\t\t\tmain_mod.__file__ = {entry_origin:?}\n"));
    out.push_str(&format!("\t\t\tmain_mod.__package__ = {entry_package:?}\n"));
    out.push_str("\t\t\tmain_mod.__spec__ = importlib.util.spec_from_loader(\n");
    out.push_str(&format!(
        "\t\t\t\t\"__main__\", loader, origin={entry_origin:?}, is_package={entry_is_package}\n"
    ));
    out.push_str("\t\t\t)\n");
    out.push_str("\t\t\tsetattr(main_mod, \"_COLLECTING\", True)\n");
    out.push_str("\t\t\tglobals().pop(\"_setup\", None)\n");
    out.push_str("\t\t\texec(code, main_mod.__dict__)\n");
    out.push('\n');
    out.push_str("\t_setup()\n");
    out.push_str("\tglobals().pop(\"_setup\", None)\n");
    out
}

fn append_module_body(out: &mut String, module: &ModuleData) {
    let protected_lines = module
        .analysis
        .as_ref()
        .map(|analysis| &analysis.multiline_string_continuation_lines);
    if let Some(docstring) = module
        .analysis
        .as_ref()
        .and_then(|analysis| analysis.docstring.as_ref())
    {
        out.push_str(module_source_indent_prefix(&module.source, protected_lines));
        out.push_str("__doc__ = ");
        out.push_str(&python_string_literal(docstring));
        out.push('\n');
    }
    let future_lines = module
        .analysis
        .as_ref()
        .map(|analysis| &analysis.future_import_lines);
    out.push_str(&module_block_body_source(
        &module.source,
        protected_lines,
        future_lines,
    ));
}

fn append_module_license_comments(
    out: &mut String,
    module: &ModuleData,
    license_headers: &HashMap<String, Vec<String>>,
) {
    if let Some(headers) = license_headers.get(&module.name) {
        for header in headers {
            out.push_str(header);
        }
    }
}

pub(crate) fn module_block_body_source(
    source: &[u8],
    protected_lines: Option<&HashSet<usize>>,
    future_lines: Option<&HashSet<usize>>,
) -> String {
    let normalized = normalize_python_newlines(&String::from_utf8_lossy(source));
    if normalized.trim().is_empty() {
        return "\tpass\n".to_string();
    }

    let lines = normalized.split('\n').collect::<Vec<_>>();
    let prefix = module_indent_prefix(&lines, protected_lines);
    let mut out = String::new();
    let mut has_statement = false;
    for (index, line) in lines.iter().enumerate() {
        if index == lines.len() - 1 && line.is_empty() {
            break;
        }
        if future_lines.is_some_and(|set| set.contains(&index)) {
            if !line.trim().is_empty() {
                out.push_str(prefix);
                out.push_str("# Hoisted by pybundler: ");
                out.push_str(line.trim());
                out.push('\n');
            }
            continue;
        }
        if !line.trim().is_empty() && !protected_lines.is_some_and(|set| set.contains(&index)) {
            out.push_str(prefix);
            has_statement |= !line.trim_start().starts_with('#');
        }
        out.push_str(line);
        out.push('\n');
    }
    if !has_statement {
        out.push_str(prefix);
        out.push_str("pass\n");
    }
    out
}

fn module_source_indent_prefix(
    source: &[u8],
    protected_lines: Option<&HashSet<usize>>,
) -> &'static str {
    let normalized = normalize_python_newlines(&String::from_utf8_lossy(source));
    let lines = normalized.split('\n').collect::<Vec<_>>();
    module_indent_prefix(&lines, protected_lines)
}

fn module_indent_prefix(lines: &[&str], protected_lines: Option<&HashSet<usize>>) -> &'static str {
    let mut space_used = false;
    for (index, line) in lines.iter().enumerate() {
        if index == lines.len() - 1 && line.is_empty() {
            break;
        }
        if line.trim().is_empty() || protected_lines.is_some_and(|set| set.contains(&index)) {
            continue;
        }
        if line.starts_with('\t') {
            return "\t";
        }
        if line.starts_with(' ') {
            space_used = true;
        }
    }
    if space_used { "    " } else { "\t" }
}

fn python_string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(ch))),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

pub(crate) fn normalize_python_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn module_package_name(module_name: &str, is_package: bool) -> String {
    if is_package {
        return module_name.to_string();
    }
    let Some(index) = module_name.rfind('.') else {
        return String::new();
    };
    module_name[..index].to_string()
}

fn module_origin(module: &ModuleData) -> String {
    let path = module.name.replace('.', "/");
    if module.is_package {
        format!("{path}/__init__.py")
    } else {
        format!("{path}.py")
    }
}
