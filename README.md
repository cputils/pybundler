# pybundler

![Badge](https://github.com/cputils/pybundler/actions/workflows/ci.yml/badge.svg)

**pybundler** is a Python module bundler. It takes a Python entry file and recursively resolves all of its local import dependencies, then generates a single self-contained Python script that can be run standalone.

Think of it like Webpack or Rollup, but for Python.

## How it works

1. You provide an entry `.py` file.
2. pybundler parses the file and follows every local `import` / `from ... import` statement, building a dependency graph with CPython-compatible package and path-entry precedence.
3. It writes every module as a readable `if __name__ == "...":` source block, followed by a lightweight import runtime that preserves package metadata and circular-import behavior.
4. The generated script runs on its own with no other files needed.

## Features

- Handles all standard import syntax: `import X`, `from X import Y`, aliases, relative imports, wildcard imports
- Supports dynamic imports via `__import__()` and `importlib.import_module()` when their arguments are compile-time constants, including static `fromlist` values
- Expands statically declared package `__all__` values for wildcard imports
- Resolves modules from the filesystem and from ZIP-based path entries regardless of archive extension or internal prefix
- Preserves CPython search precedence across regular packages, namespace packages, modules, directories, and ZIP path entries
- Exclude specific packages from bundling (they remain normal runtime imports)
- Skip individual imports with a `# no-bundle` comment directive
- Force-bundle packages listed as external with a `# bundle` comment directive
- Bundle imports resolved through interpreter `sys.path` only when marked with a `# bundle` comment directive by default; this requirement can be disabled with an option
- Safety limit on the number of modules to bundle (prevents runaway graphs)
- Automatically collects and embeds license texts from third-party packages
- Supports namespace packages by synthesizing missing `__init__.py` parents
- Queries Python interpreters to discover `sys.path` for accurate module resolution
- Supports UTF-8 BOMs and PEP 263 source declarations for UTF-8, ASCII, and Latin-1
- Hoists bundled modules' `__future__` imports so the readable combined script remains valid Python
- Removes unused imports from bundled modules, except imports marked with `# bundle`
- Formats the bundled output with Ruff

Native extension modules, sourceless bytecode modules, and source files using interpreter-registered codecs other than UTF-8, ASCII, or Latin-1 remain runtime dependencies because reproducing them requires the target Python interpreter or platform.

## Usage

pybundler is a Rust library. Add it to your `Cargo.toml`:

```toml
[dependencies]
pybundler = { git = "https://github.com/cputils/pybundler", tag = "<version>" }
```

Then use it in your code:

```rust
use pybundler::{bundle_file, BundleOptions};

let result = bundle_file("src/main.py", BundleOptions::default())?;
std::fs::write("bundled.py", result.code)?;
```

### Options

| Option                     | Description                                                    | Default |
| -------------------------- | -------------------------------------------------------------- | ------- |
| `external`                 | Package names to keep as runtime imports (not bundled)         | `[]`    |
| `max_imported_modules`     | Maximum number of modules to bundle                            | `2048`  |
| `interpreter`              | Python interpreters used to discover opt-in `sys.path` imports | `[]`    |
| `require_bundle_directive` | Require `# bundle` for imports resolved through `sys.path`     | `true`  |
| `tree_shaking`             | Remove unused imports unless marked with `# bundle`            | `true`  |
| `format`                   | Format bundled output                                          | `false` |

## License

MIT
