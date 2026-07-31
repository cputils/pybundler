use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use pybundler::{BundleOptions, bundle_file};

#[cfg(not(windows))]
const DEFAULT_INTERPRETERS: &[&str] = &["python3", "python", "pypy3", "pypy"];
#[cfg(windows)]
const DEFAULT_INTERPRETERS: &[&str] = &["py", "python", "python3", "pypy3", "pypy"];

fn default_interpreters() -> Vec<String> {
    DEFAULT_INTERPRETERS
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// Bundle a Python program and its local dependencies into a single script.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// Python entry file to bundle.
    entry: PathBuf,

    /// Write the bundle to this file instead of standard output.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Keep this top-level package as a runtime import. May be repeated.
    #[arg(short, long)]
    external: Vec<String>,

    /// Maximum number of imported modules to bundle.
    #[arg(long, default_value_t = 2048)]
    max_imported_modules: usize,

    /// Python interpreter used to discover sys.path. May be repeated.
    #[arg(short, long, default_values_t = default_interpreters())]
    interpreter: Vec<String>,

    /// Bundle imports discovered through sys.path without a # bundle directive.
    #[arg(long)]
    no_require_bundle_directive: bool,

    /// Keep unused imports in bundled modules.
    #[arg(long)]
    no_tree_shaking: bool,

    /// Format the bundled output with Ruff.
    #[arg(long)]
    format: bool,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pybundler: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let entry = cli.entry.to_string_lossy();
    let result = bundle_file(
        &entry,
        BundleOptions {
            external: cli.external,
            max_imported_modules: cli.max_imported_modules,
            interpreter: cli.interpreter,
            require_bundle_directive: !cli.no_require_bundle_directive,
            tree_shaking: !cli.no_tree_shaking,
            format: cli.format,
        },
    )?;

    if let Some(output) = cli.output {
        fs::write(&output, result.code)
            .map_err(|error| format!("write output file {}: {error}", output.display()))?;
    } else {
        print!("{}", result.code);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_platform_interpreters_by_default() {
        let cli = Cli::try_parse_from(["pybundler", "main.py"]).expect("parse CLI arguments");

        assert_eq!(cli.interpreter, default_interpreters());
    }

    #[test]
    fn explicit_interpreters_replace_platform_defaults() {
        let cli = Cli::try_parse_from(["pybundler", "main.py", "--interpreter", "custom-python"])
            .expect("parse CLI arguments");

        assert_eq!(cli.interpreter, ["custom-python"]);
    }
}
