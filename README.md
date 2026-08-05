# pybundler

![Badge](https://github.com/cputils/pybundler/actions/workflows/ci.yml/badge.svg)

English | [日本語](README.ja.md)

**pybundler** bundles a Python entry file and its local dependencies into a single, standalone Python script.

## Quick start

Install the `pybundler` command from the repository:

```sh
cargo install --git https://github.com/cputils/pybundler pybundler
```

Bundle a program:

```sh
pybundler src/main.py --output bundled.py
python bundled.py
```

Without `--output`, the generated script is written to standard output:

```sh
pybundler src/main.py > bundled.py
```

Use `pybundler --help` to see every option.

## Detailed reference

### How bundling works

1. pybundler parses the entry file and recursively follows its local imports.
2. It resolves those imports using CPython-compatible package and path-entry precedence.
3. It emits each bundled module as a readable `if __name__ == "...":` source block and adds a lightweight import runtime.
4. The resulting script runs without the bundled source files.

The generated runtime preserves package metadata and circular-import behavior. Missing `__init__.py` parents for namespace packages are synthesized when necessary.

### Supported imports and module resolution

pybundler supports:

- `import`, `from ... import`, aliases, relative imports, and wildcard imports
- `__import__()` and `importlib.import_module()` when their arguments are compile-time constants, including static `fromlist` values
- wildcard imports from packages with a statically declared `__all__`
- regular packages, namespace packages, modules, directories, and ZIP-based path entries
- ZIP path entries with any archive extension and with an internal path prefix
- CPython search precedence across all supported module and path-entry types

By default, resolution starts at the entry file's directory. The CLI also tries common Python commands to discover additional `sys.path` entries, ignoring commands that are not installed or cannot run. It tries `python3`, `python`, `pypy3`, and `pypy` on Unix-like systems; on Windows it also tries the `py` launcher first. Passing one or more `--interpreter` values replaces these defaults.

An import resolved from an interpreter's `sys.path` must have a `# bundle` directive at the project boundary unless `--no-require-bundle-directive` is used. Once admitted, its transitive imports are bundled without additional directives.

Imports whose top-level package is passed with `--external` remain runtime imports. A `# bundle` directive on an individual import overrides that setting, while `# no-bundle` prevents that import from being bundled.

### Import controls

Keep a package as a normal runtime dependency instead of bundling it:

```sh
pybundler src/main.py --output bundled.py --external numpy
```

Bundle an installed package by marking its import with `# bundle`:

```python
import some_package  # bundle
```

Use `--interpreter <COMMAND>` when you need to select a particular Python installation instead of the platform defaults.

Add `# no-bundle` to an import when it should remain a normal runtime import:

```python
import another_package  # no-bundle
```

### Source processing and output

- Unused imports are removed by default; imports marked with `# bundle` are preserved.
- `--no-tree-shaking` keeps unused imports.
- Bundled modules' `__future__` imports are hoisted so the combined script remains valid Python.
- UTF-8 BOMs and PEP 263 declarations for UTF-8, ASCII, and Latin-1 are supported. Source is emitted as UTF-8, and encoding declarations are normalized accordingly.
- License texts discovered in bundled third-party packages are embedded automatically.
- `--format` formats the generated script with Ruff.
- `--max-imported-modules` limits dependency-graph expansion and defaults to `2048`.

Native extension modules, sourceless bytecode modules, and source files using interpreter-registered codecs other than UTF-8, ASCII, or Latin-1 remain runtime dependencies. Reproducing them requires the target Python interpreter or platform.

### CLI options

Options marked as repeatable may be supplied more than once.

| Option                           | Description                                                | Default                  |
| -------------------------------- | ---------------------------------------------------------- | ------------------------ |
| `-o, --output <FILE>`            | Write the bundle to a file instead of standard output      | standard output          |
| `-e, --external <PACKAGE>`       | Keep a top-level package as a runtime import; repeatable   | none                     |
| `--max-imported-modules <COUNT>` | Limit the number of imported modules bundled               | `2048`                   |
| `-i, --interpreter <COMMAND>`    | Discover `sys.path` using a Python interpreter; repeatable | common platform commands |
| `--no-require-bundle-directive`  | Bundle imports found through `sys.path` without `# bundle` | disabled                 |
| `--no-tree-shaking`              | Keep unused imports in bundled modules                     | disabled                 |
| `--format`                       | Format the generated bundle with Ruff                      | disabled                 |

### Rust library

Add pybundler to `Cargo.toml`:

```toml
[dependencies]
pybundler = { git = "https://github.com/cputils/pybundler", tag = "<version>" }
```

Then call `bundle_file`:

```rust
use pybundler::{BundleOptions, bundle_file};

let result = bundle_file("src/main.py", BundleOptions::default())?;
std::fs::write("bundled.py", result.code)?;
```

`BundleOptions` provides the library equivalents of the CLI settings. Unlike the CLI, the library does not select interpreters by default:

| Field                      | Description                                                | Default |
| -------------------------- | ---------------------------------------------------------- | ------- |
| `external`                 | Top-level packages to keep as runtime imports              | `[]`    |
| `max_imported_modules`     | Maximum number of imported modules to bundle               | `2048`  |
| `interpreter`              | Python interpreters used to discover `sys.path`            | `[]`    |
| `require_bundle_directive` | Require `# bundle` for imports resolved through `sys.path` | `true`  |
| `tree_shaking`             | Remove unused imports unless marked with `# bundle`        | `true`  |
| `format`                   | Format the bundled output with Ruff                        | `false` |

`bundle_file` returns a `BundleResult` containing the generated `code`, entry-file and entry-module information, and metadata for every bundled module.

## License

MIT
