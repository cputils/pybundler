use std::collections::HashMap;

use crate::bundler::ModuleData;

pub(crate) fn generate_bundle_code(
    entry: &ModuleData,
    modules: &HashMap<String, ModuleData>,
    license_headers: &HashMap<String, Vec<String>>,
    force_runtime: bool,
) -> String {
    let mut names = modules.keys().cloned().collect::<Vec<_>>();
    names.sort();
    let license_comments = names
        .iter()
        .filter_map(|name| license_headers.get(name))
        .flatten()
        .cloned()
        .collect::<String>();

    if names.len() == 1 && !force_runtime {
        let mut source = module_source(entry);
        if !license_comments.is_empty() {
            if !source.ends_with('\n') {
                source.push('\n');
            }
            source.push('\n');
            source.push_str(&license_comments);
        }
        return source;
    }

    let mut out = license_comments;
    out.push_str("if __name__ == \"__main__\":\n");
    out.push_str("\tdef _setup():\n");
    out.push_str("\t\timport importlib.abc\n");
    out.push_str("\t\timport importlib.machinery\n");
    out.push_str("\t\timport importlib.util\n");
    out.push_str("\t\timport sys\n");
    out.push('\n');
    out.push_str("\t\t# The following runtime code is part of pybundler.\n");
    out.push_str("\t\t# https://github.com/cputils/pybundler\n");
    out.push_str("\t\t# SPDX-License-Identifier: CC0-1.0\n");
    out.push('\n');
    out.push_str("\t\tmodules_info = {\n");
    for name in &names {
        let Some(mod_data) = modules.get(name) else {
            continue;
        };
        let source = if mod_data.synthetic {
            "None".to_string()
        } else {
            python_string_literal(&module_source(mod_data))
        };
        let origin = if mod_data.synthetic {
            "None".to_string()
        } else {
            python_string_literal(&module_origin(mod_data))
        };
        let is_package = if mod_data.is_package { "True" } else { "False" };
        let synthetic = if mod_data.synthetic { "True" } else { "False" };
        out.push_str(&format!(
            "\t\t\t{}: ({source}, {origin}, {is_package}, {synthetic}),\n",
            python_string_literal(name)
        ));
    }
    out.push_str("\t\t}\n");
    out.push('\n');

    out.push_str("\t\tclass Loader(importlib.abc.Loader):\n");
    out.push_str("\t\t\tdef exec_module(self, module):\n");
    out.push_str("\t\t\t\tsource, origin, _, _ = modules_info[module.__name__]\n");
    out.push_str(
        "\t\t\t\texec(compile(source, origin, \"exec\", dont_inherit=True), module.__dict__)\n",
    );
    out.push('\n');
    out.push_str("\t\t\tdef get_filename(self, fullname):\n");
    out.push_str("\t\t\t\treturn modules_info[fullname][1]\n");
    out.push('\n');
    out.push_str("\t\t\tdef get_source(self, fullname):\n");
    out.push_str("\t\t\t\treturn modules_info[fullname][0]\n");
    out.push('\n');
    out.push_str("\t\t\tdef get_code(self, fullname):\n");
    out.push_str("\t\t\t\tsource, origin, _, _ = modules_info[fullname]\n");
    out.push_str("\t\t\t\treturn compile(source, origin, \"exec\", dont_inherit=True)\n");
    out.push('\n');
    out.push_str("\t\t\tdef is_package(self, fullname):\n");
    out.push_str("\t\t\t\treturn modules_info[fullname][2]\n");
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
    out.push_str("\t\t\t\tif info[3]:\n");
    out.push_str(
        "\t\t\t\t\tspec = importlib.util.spec_from_loader(fullname, None, is_package=True)\n",
    );
    out.push_str("\t\t\t\t\tspec.submodule_search_locations.append(\n");
    out.push_str("\t\t\t\t\t\tfullname.replace(\".\", \"/\")\n");
    out.push_str("\t\t\t\t\t)\n");
    out.push_str("\t\t\t\t\treturn spec\n");
    out.push_str(
        "\t\t\t\treturn importlib.util.spec_from_loader(fullname, loader, is_package=info[2])\n",
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
    out.push_str(&format!(
        "\t\tsource, origin, _, _ = modules_info[{}]\n",
        python_string_literal(&entry.name)
    ));
    out.push_str("\t\tglobals().pop(\"_setup\", None)\n");
    out.push_str("\t\texec(compile(source, origin, \"exec\", dont_inherit=True), globals())\n");
    out.push('\n');
    out.push_str("\t_setup()\n");
    out.push_str("\tglobals().pop(\"_setup\", None)\n");
    out
}

fn module_source(module: &ModuleData) -> String {
    normalize_python_newlines(&String::from_utf8_lossy(&module.source))
}

fn module_origin(module: &ModuleData) -> String {
    let path = module.name.replace('.', "/");
    if module.is_package {
        format!("{path}/__init__.py")
    } else {
        format!("{path}.py")
    }
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
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ch if ch <= '\u{ff}' && ch.is_control() => {
                out.push_str(&format!("\\x{:02x}", u32::from(ch)));
            }
            ch if ch.is_control() => {
                let value = u32::from(ch);
                if value <= 0xffff {
                    out.push_str(&format!("\\u{value:04x}"));
                } else {
                    out.push_str(&format!("\\U{value:08x}"));
                }
            }
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
    let Some(idx) = module_name.rfind('.') else {
        return String::new();
    };
    module_name[..idx].to_string()
}
