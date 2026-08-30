use std::path::{Path, PathBuf};

use ra_ap_syntax::Edition;

#[path = "cfg.rs"]
mod cfg;
#[path = "directive.rs"]
mod directive;
#[path = "edit.rs"]
mod edit;
#[path = "engine.rs"]
mod engine;
#[path = "include.rs"]
mod include;
#[path = "macro_rules.rs"]
mod macro_rules;
#[path = "module_resolver.rs"]
mod module_resolver;
#[path = "proc_macro.rs"]
mod proc_macro;
#[path = "proc_macro_discovery.rs"]
mod proc_macro_discovery;
#[path = "source.rs"]
mod source;

const DEFAULT_MAX_SOURCE_FILES: usize = 2048;

/// Rust language edition used to parse bundled sources.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RustEdition {
    Edition2015,
    Edition2018,
    Edition2021,
    #[default]
    Edition2024,
}

impl From<RustEdition> for Edition {
    fn from(value: RustEdition) -> Self {
        match value {
            RustEdition::Edition2015 => Edition::Edition2015,
            RustEdition::Edition2018 => Edition::Edition2018,
            RustEdition::Edition2021 => Edition::Edition2021,
            RustEdition::Edition2024 => Edition::Edition2024,
        }
    }
}

/// Configuration for [`bundle_file`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleOptions {
    /// Rust edition used by the parser.
    pub edition: RustEdition,
    /// Maximum number of module and include files expanded, excluding the entry.
    pub max_source_files: usize,
    /// Whether static `include!`, `include_str!`, and `include_bytes!` calls are expanded.
    pub inline_includes: bool,
    /// Top-level modules to retain as out-of-line runtime/build-time dependencies.
    /// A `// bundle` directive on a declaration overrides this list.
    pub external: Vec<String>,
    /// Compile-time environment values available to statically evaluated `env!` calls.
    pub environment: Vec<(String, String)>,
    /// Compiled procedural-macro libraries that override automatic discovery.
    pub proc_macros: Vec<ProcMacroDylib>,
}

impl Default for BundleOptions {
    fn default() -> Self {
        Self {
            edition: RustEdition::default(),
            max_source_files: DEFAULT_MAX_SOURCE_FILES,
            inline_includes: true,
            external: Vec::new(),
            environment: Vec::new(),
            proc_macros: Vec::new(),
        }
    }
}

/// A compiled procedural-macro library and the crate path used to invoke it.
///
/// Explicit entries override matching libraries discovered from Cargo build
/// artifacts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcMacroDylib {
    /// First path segment used for qualified macro invocations.
    pub crate_name: String,
    /// Path to a host proc-macro dylib compiled by the same `rustc` as rsbundler.
    pub dylib_path: PathBuf,
}

/// How a source dependency entered the generated bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundledSourceKind {
    Entry,
    Module,
    Include,
    IncludeStr,
    IncludeBytes,
}

/// Metadata for a source file used by a bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundledSource {
    /// Canonical absolute source path.
    pub file_path: String,
    /// Rust module path at the expansion site. The entry uses `crate`.
    pub module_path: String,
    /// The construct that introduced this file.
    pub kind: BundledSourceKind,
}

/// Result returned by [`bundle_file`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleResult {
    /// Generated single-file Rust source.
    pub code: String,
    /// Canonical absolute path of the entry crate root.
    pub entry_file: String,
    /// Every source file used, in deterministic discovery order.
    pub bundled_source_list: Vec<BundledSource>,
}

/// Bundles a Rust crate root and its local source-file dependencies.
///
/// The bundler uses an embedded rust-analyzer parser and proc-macro host and
/// never invokes `cargo`, `rustc`, or another external executable. Configured
/// procedural macros are discovered from Cargo build artifacts or loaded from
/// configured dylibs and expanded before out-of-line module declarations are
/// converted to inline modules. Static include macros are embedded by default.
/// All source text outside replacement ranges is retained byte-for-byte.
/// Dependencies marked `// no-bundle`, configured as external, or not safely
/// resolvable by static analysis are retained unchanged. `// bundle` forces one
/// dependency to be expanded and turns a resolution failure into an error.
///
/// # Errors
///
/// An error is returned for invalid input or Rust syntax, invalid cfg options,
/// conflicting directives, source read failures, a source-file limit overflow,
/// or an explicitly forced dependency that cannot be resolved. This includes a
/// cycle reached through a forced dependency. Unforced unresolved or cyclic
/// dependencies remain in the output.
pub fn bundle_file(
    entry_file: impl AsRef<Path>,
    options: BundleOptions,
) -> Result<BundleResult, String> {
    engine::bundle_file(entry_file.as_ref(), options)
}
