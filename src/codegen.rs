use std::collections::HashMap;
use std::path::Path;

use crate::bundler::ModuleData;

pub(crate) fn generate_bundle_code(
    entry: &ModuleData,
    modules: &HashMap<String, ModuleData>,
    license_headers: &HashMap<String, Vec<String>>,
) -> String {
    let mut names = modules.keys().cloned().collect::<Vec<_>>();
    names.sort();

    let mut out = String::new();

    if names.len() == 1 {
        if let Some(headers) = license_headers.get(&entry.name) {
            for header in headers {
                out.push_str(header);
            }
        }
        out.push_str(&normalize_python_newlines(&String::from_utf8_lossy(
            &entry.source,
        )));
        return out;
    }

    if let Some(headers) = license_headers.get(&entry.name) {
        for header in headers {
            out.push_str(header);
        }
    }
    out.push_str("if __name__ == \"__main__\" and globals().get(\"_COLLECTING\"):\n");
    out.push_str(&module_block_body_source(
        &entry.source,
        entry.analysis_lines().as_ref(),
    ));
    out.push('\n');

    for name in &names {
        if *name == entry.name {
            continue;
        }
        let Some(mod_data) = modules.get(name) else {
            continue;
        };
        if let Some(headers) = license_headers.get(name) {
            for header in headers {
                out.push_str(header);
            }
        }
        out.push_str("if __name__ == ");
        out.push_str(&format!("{name:?}"));
        out.push_str(":\n");
        out.push_str(&module_block_body_source(
            &mod_data.source,
            mod_data.analysis_lines().as_ref(),
        ));
        out.push('\n');
    }

    out.push_str("if not globals().get(\"_COLLECTING\"):\n");
    out.push_str("\t# The following runtime code is part of pybundler.\n");
    out.push_str("\t# https://github.com/cputils/pybundler\n");
    out.push_str("\t# SPDX-License-Identifier: CC0-1.0\n");
    out.push('\n');
    out.push_str("\tdef _setup():\n");
    out.push_str("\t\timport importlib.abc\n");
    out.push_str("\t\timport importlib.util\n");
    out.push_str("\t\timport sys\n");
    out.push('\n');

    out.push_str("\t\tmodules_info = {\n");
    for name in &names {
        if let Some(mod_data) = modules.get(name) {
            let origin = file_basename(&mod_data.file_path);
            let is_pkg = if mod_data.is_package { "True" } else { "False" };
            out.push_str(&format!("\t\t\t{name:?}: ({origin:?}, {is_pkg}),\n"));
        }
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

    out.push_str("\t\tclass Finder(importlib.abc.MetaPathFinder):\n");
    out.push_str("\t\t\tdef find_spec(self, fullname, path=None, target=None):\n");
    out.push_str("\t\t\t\tinfo = modules_info.get(fullname)\n");
    out.push_str("\t\t\t\tif info is None:\n");
    out.push_str("\t\t\t\t\treturn None\n");
    out.push_str("\t\t\t\treturn importlib.util.spec_from_loader(\n");
    out.push_str("\t\t\t\t\tfullname,\n");
    out.push_str("\t\t\t\t\tloader,\n");
    out.push_str("\t\t\t\t\torigin=info[0],\n");
    out.push_str("\t\t\t\t\tis_package=info[1],\n");
    out.push_str("\t\t\t\t)\n");
    out.push('\n');

    out.push_str("\t\tloader = Loader()\n");
    out.push_str("\t\tfinder = Finder()\n");
    out.push_str("\t\tif not any(isinstance(x, Finder) for x in sys.meta_path):\n");
    out.push_str("\t\t\tsys.meta_path.insert(0, finder)\n");
    out.push('\n');

    let entry_package = module_package_name(&entry.name, entry.is_package);
    let entry_origin = format!("{:?}", file_basename(&entry.file_path));

    out.push_str("\t\tif __name__ == \"__main__\":\n");
    out.push_str("\t\t\tmain_mod = sys.modules[\"__main__\"]\n");
    out.push_str(&format!("\t\t\tmain_mod.__file__ = {entry_origin}\n"));
    out.push_str("\t\t\tmain_mod.__package__ = ");
    if entry_package.is_empty() {
        out.push_str("\"\"\n");
    } else {
        out.push_str(&format!("{entry_package:?}\n"));
    }
    out.push_str("\t\t\tmain_mod.__spec__ = importlib.util.spec_from_loader(\n");
    out.push_str(&format!(
        "\t\t\t\t\"__main__\", loader, origin={entry_origin}\n"
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

impl ModuleData {
    pub(crate) fn analysis_lines(&self) -> Option<std::collections::HashSet<usize>> {
        self.analysis
            .as_ref()
            .map(|analysis| analysis.multiline_string_continuation_lines.clone())
    }
}

pub(crate) fn module_block_body_source(
    source: &[u8],
    protected_lines: Option<&std::collections::HashSet<usize>>,
) -> String {
    let normalized = normalize_python_newlines(&String::from_utf8_lossy(source));
    if normalized.trim().is_empty() {
        return "\tpass\n".to_string();
    }

    let lines = normalized.split('\n').collect::<Vec<_>>();
    let prefix = module_indent_prefix(&lines, protected_lines);
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i == lines.len() - 1 && line.is_empty() {
            break;
        }
        if !line.trim().is_empty() && !protected_lines.is_some_and(|set| set.contains(&i)) {
            out.push_str(prefix);
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn module_indent_prefix(
    lines: &[&str],
    protected_lines: Option<&std::collections::HashSet<usize>>,
) -> &'static str {
    let mut space_used = false;
    for (i, line) in lines.iter().enumerate() {
        if i == lines.len() - 1 && line.is_empty() {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        if protected_lines.is_some_and(|set| set.contains(&i)) {
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

pub(crate) fn normalize_python_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

pub(crate) fn module_package_name(module_name: &str, is_package: bool) -> String {
    if is_package {
        return module_name.to_string();
    }
    let Some(idx) = module_name.rfind('.') else {
        return String::new();
    };
    module_name[..idx].to_string()
}

fn file_basename(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| path.display().to_string(), ToString::to_string)
}
