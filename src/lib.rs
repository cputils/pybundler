//! pybundler (https://github.com/cputils/pybundler)
//!
//! Use [`bundle_file`] to bundle an entry Python file and its local imports into a single script.

mod analyzer;
mod bundler;
mod codegen;
mod licenses;
mod module_graph;
mod resolver;
mod sys_paths;
mod tree_shaking;

pub use bundler::{BundleOptions, BundleResult, BundledModule, bundle_file};
