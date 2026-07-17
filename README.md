# pybundler

![Badge](https://github.com/cputils/pybundler/actions/workflows/ci.yml/badge.svg)

**pybundler** is a Python module bundler. It takes a Python entry file and recursively resolves all of its local import dependencies, then generates a single self-contained Python script that can be run standalone.

Think of it like Webpack or Rollup, but for Python.

## How it works

1. You provide an entry `.py` file.
2. pybundler parses the file and follows every local `import` / `from ... import` statement, building a dependency graph.
3. It inlines all local modules into a single output script, complete with a lightweight import runtime so `__name__`, `__package__`, and `sys.modules` work correctly.
4. The generated script runs on its own with no other files needed.

## Features

- Handles all standard import syntax: `import X`, `from X import Y`, aliases, relative imports, wildcard imports
- Supports dynamic imports via `__import__()` and `importlib.import_module()` (when the argument is a compile-time constant)
- Resolves modules from the filesystem and from `.zip` / `.egg` archives
- Exclude specific packages from bundling (they remain normal runtime imports)
- Skip individual imports with a `# no-bundle` comment directive
- Force-bundle externals with a `# bundle` comment directive
- Safety limit on the number of modules to bundle (prevents runaway graphs)
- Automatically collects and embeds license texts from third-party packages
- Supports namespace packages by synthesizing missing `__init__.py` parents
- Queries Python interpreters to discover `sys.path` for accurate module resolution
- Removes unused imports from bundled modules
- Formats the bundled output with Ruff

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

| Option                 | Description                                            | Default |
| ---------------------- | ------------------------------------------------------ | ------- |
| `external`             | Package names to keep as runtime imports (not bundled) | `[]`    |
| `max_imported_modules` | Maximum number of modules to bundle                    | `2048`  |
| `interpreter`          | Python interpreter paths used to discover `sys.path`   | `[]`    |
| `tree_shaking`         | Remove unused imports from bundled output              | `true`  |
| `format`               | Format bundled output                                  | `false` |

## License

MIT
