//! Bundle a Rust crate's local source dependencies into one source file.
//!
//! [`bundle_file`] expands out-of-line modules and static `include!`,
//! `include_str!`, and `include_bytes!` invocations without invoking `cargo`,
//! `rustc`, or any other external process. `// bundle` and `// no-bundle`
//! directives can force or suppress individual expansions.

mod bundler;

pub use bundler::{
    BundleOptions, BundleResult, BundledSource, BundledSourceKind, RustEdition, bundle_file,
};
