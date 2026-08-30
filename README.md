# rsbundler

English | [日本語](README.ja.md)

**rsbundler** bundles a Rust crate root and its local source-file dependencies into one Rust source file. Its parser is linked into the executable; bundling never launches `rustc`, `cargo`, `rustfmt`, or another external command.

## Quick start

Install the command from the repository:

```sh
cargo install --git https://github.com/cputils/rsbundler rsbundler
```

Bundle a binary crate root:

```sh
rsbundler src/main.rs --output bundled.rs
```

Without `--output`, generated source is written to standard output:

```sh
rsbundler src/main.rs > bundled.rs
```

The `rsbundler` executable does not need a Rust toolchain at runtime. A Rust compiler is, naturally, still required later if the generated `.rs` source is to be compiled.

## How it works

1. rsbundler parses the crate root with the rust-analyzer syntax parser embedded in the executable.
2. It discovers compiled procedural-macro libraries in the Cargo project's existing build artifacts and expands their attribute, derive, and function-like macros in process.
3. It follows out-of-line `mod name;` declarations using Rust's file-resolution rules.
4. It replaces each declaration's semicolon with an inline module body while retaining the declaration's visibility, attributes, comments, and source text.
5. By default, it also embeds files referenced by static `include!`, `include_str!`, and `include_bytes!` calls.

Source is not printed back from an abstract syntax tree. Only the exact ranges that need expansion are replaced, so comments, raw strings, whitespace, and line endings elsewhere are retained.

## Supported source resolution

- `name.rs` and `name/mod.rs`, including ambiguity detection when both exist
- nested out-of-line modules in flat, `mod.rs`, and inline modules
- raw module identifiers such as `mod r#type;`
- `#[path = "..."]` and active `cfg_attr(..., path = "...")`, including their distinct nested-module rules
- nested static `include!` files containing Rust items or one expression
- `include_str!` with arbitrary UTF-8 text and `include_bytes!` with arbitrary bytes
- literal, `concat!`, `env!`, and `stringify!` include paths without comments or internal whitespace
- string, character, integer (including non-decimal and suffixed forms), float, boolean, and negative-number literals inside `concat!`
- qualified standard macros and explicit aliases such as `std::include_str!` and `use std::include_str as embedded`
- direct `file!`, `line!`, and `column!` calls, including explicit standard aliases, replaced with their original values before source is moved
- statically parseable module and include dependencies in `macro_rules!` transcribers
- attribute, derive, and function-like procedural macros discovered from Cargo build artifacts or explicitly overridden with host dylibs
- target-independent expansion of `cfg`, `all`, `any`, `not`, and nested `cfg_attr` branches
- UTF-8 BOMs and shebangs in source files
- transactional cycle detection and a configurable dependency-file limit

External crates and `use` declarations are left unchanged. rsbundler deliberately does not run Cargo metadata or try to copy third-party crates into the source file.

## Procedural macros

No proc-macro option is normally needed. Build the Cargo project first, then bundle its crate root:

```sh
cargo build
rsbundler src/main.rs --output bundled.rs
```

Starting at the entry file, rsbundler finds the enclosing package and workspace manifests, reads dependency renames, and searches their existing Cargo artifact directories. It honors `CARGO_TARGET_DIR` and `[build].target-dir`; otherwise it searches package and workspace `target` directories, preferring the debug profile before release and custom profiles. It does not invoke Cargo or rustc and does not build missing artifacts.

For a nonstandard layout or to override the automatically selected artifact, pass an already compiled proc-macro dylib as `CRATE=DYLIB`. The crate name is the first path segment used at invocation sites, and the option is repeatable:

```sh
rsbundler src/main.rs \
  --proc-macro my_macros=target/debug/deps/libmy_macros-abc123.so \
  --output bundled.rs
```

rsbundler reads the macro exports from each library and expands registered attribute macros, custom derives, and function-like macros before ordinary source dependency discovery. Macro output is processed recursively, so a generated `mod`, `include!`, or another registered procedural macro participates in the same bundle. Qualified invocations such as `#[my_macros::transform]` are matched by the discovered dependency name or configured override. An unqualified name is expanded when it identifies exactly one loaded macro of that kind; qualify collisions explicitly.

The dylib must target the host platform and must have been built by exactly the same `rustc` version as the rsbundler executable because Rust's proc-macro dylib ABI is unstable. An explicitly configured ABI mismatch is rejected with the expected and actual compiler versions; incompatible automatically discovered candidates are skipped. rsbundler never starts Cargo, rustc, or an external proc-macro server. The proc-macro host, artifact discovery, and their Rust dependencies are linked into the rsbundler binary; only project dylibs are loaded at runtime. Procedural macros are native code and run with the permissions of rsbundler, so bundle only trusted projects and artifacts.

`--env KEY=VALUE` values are also provided to procedural macros. Their current directory is the entry source's directory. Macros selected indirectly by `cfg_attr`, compiler-provided derives, and macros that require full compiler name resolution remain outside this syntactic expansion model; unexpanded attributes retain the existing conservative module behavior.

## Bundle control

Like pybundler, rsbundler recognizes `bundle` and `no-bundle` in a regular comment on the same line or the immediately preceding standalone comment line:

```rust
mod embedded; // bundle

// no-bundle
mod generated_at_build_time;

const SCHEMA: &str = include_str!(SCHEMA_PATH); // no-bundle
```

`// no-bundle` always retains the declaration or macro call unchanged. If moving that construct would change a relative path or source location, its nearest enclosing file dependency is retained too. `// bundle` forces static expansion, overrides `--external`, and reports an error if the dependency cannot be resolved. An inner `// no-bundle` still takes precedence over forcing an enclosing dependency. Using both directives on one dependency is an error. Without either directive, rsbundler expands dependencies it can resolve safely and transactionally rolls back unsupported, missing, ambiguous, dynamically computed, or cyclic dependencies instead of failing.

`-e, --external <MODULE>` retains a top-level module and its subtree. It is repeatable and accepts `name` or `crate::name`; a `// bundle` declaration admits that module and its transitive local modules.

### Conditional compilation

rsbundler never selects a compilation target and does not use the cfg values with which its own executable was compiled. It retains the original conditional attributes and expands every dependency branch that can be resolved statically. Predicates that are inherently false, such as `any()`, are skipped without reading their dependencies.

When `cfg_attr(..., path = "...")` can select different files for one module, rsbundler emits mutually exclusive `#[cfg(...)]` inline-module branches for every selected path and for the default module path. This produces one bundle that the Rust compiler can use with any cfg set. A missing, ambiguous, cyclic, generated, or otherwise unsafe candidate remains as the original out-of-line declaration only under its corresponding condition; other candidates are still expanded. If multiple path attributes can be active simultaneously, that overlap branch is retained unchanged so rustc keeps control of its own overlap diagnostics and precedence behavior. Nested `cfg_attr`, conditional attribute macros, path-changing inline ancestors, and each candidate's distinct child-module base are handled in the same way.

## Accuracy boundaries

Some behavior cannot be reproduced completely without full compiler name resolution:

- a `macro_rules!` transcriber containing metavariables can choose dependencies from its invocation and surrounding scope;
- an unconfigured or conditionally selected procedural macro can generate dependencies or distinguish an inline module from an out-of-line declaration;
- an arbitrary user or external macro can expand to an include or a source-location-sensitive built-in;
- a location-sensitive macro hidden in the entry file has no enclosing file dependency that rsbundler can retain.

User-defined and imported macro names are tracked conservatively, including discovered `#[macro_export]` definitions and direct or conditional `macro_use` attributes. Unqualified include, static-path, or location macro calls are retained if shadowing is possible; qualified standard paths and standard aliases are expanded only when their `std` or `core` prefix is available and unshadowed. Parseable dependency constructs in `macro_rules!` transcribers are expanded, while context-dependent transcribers cause the enclosing dependency to be retained when moving them would be unsafe. Modules with an unknown attribute macro are likewise retained automatically. Add `// bundle` only when you are asserting that a recognized construct is safe to inline.

Direct `file!`, `line!`, and `column!` values are preserved exactly. If a location macro or include call is nested in another macro's input, rsbundler retains the enclosing module or include whenever moving it could change the source position or relative-path base. At the crate entry there is no parent dependency to retain, so a hidden location macro causes rsbundler to leave the entry unchanged; compiling that returned text under a different file name can still change a hidden `file!` result. Fully reproducing that case requires expanding the user macro.

Retained `mod` and include constructs still depend on their source files. rsbundler retains a larger enclosing dependency whenever partial inlining would change their relative-path base, but the top-level retained paths are interpreted from the generated file's location. Put the output beside the entry file or preserve the required relative layout when using `no-bundle`, `external`, or automatic retention. A fully expanded result has no such local build-time source dependency.

pybundler's interpreter/sys.path boundary option has no Rust equivalent because every Rust file module is declared explicitly. Python import tree shaking is also unnecessary—the Rust compiler performs dead-code elimination—and source-level removal would be unsafe. Formatting is not performed because invoking `rustfmt` would violate the no-external-runtime guarantee. Package-license embedding is not applicable because rsbundler does not copy Cargo dependencies.

## CLI options

| Option                       | Description                                                       | Default         |
| ---------------------------- | ----------------------------------------------------------------- | --------------- |
| `-o, --output <FILE>`        | Write to a file instead of standard output                        | standard output |
| `--edition <EDITION>`        | Parse as Rust `2015`, `2018`, `2021`, or `2024`                   | `2024`          |
| `--max-source-files <COUNT>` | Limit expanded module and include files, excluding the entry      | `2048`          |
| `-e, --external <MODULE>`    | Retain a top-level module; repeatable                             | none            |
| `--env <KEY=VALUE>`          | Set an `env!` value and a procedural-macro environment override   | process env     |
| `--proc-macro <CRATE=DYLIB>` | Override an automatically discovered procedural macro; repeatable | automatic       |
| `--no-inline-includes`       | Leave all three built-in include macro forms in generated code    | disabled        |

Use `rsbundler --help` for the command's complete help text.

## Rust library

Add rsbundler to `Cargo.toml`:

```toml
[dependencies]
rsbundler = { git = "https://github.com/cputils/rsbundler", tag = "<version>" }
```

Then call `bundle_file`:

```rust
use rsbundler::{BundleOptions, bundle_file};

let result = bundle_file("src/main.rs", BundleOptions::default())?;
std::fs::write("bundled.rs", result.code)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`BundleResult` also contains the canonical entry path and deterministic metadata for every module or include file used in the bundle.

`BundleOptions` exposes the corresponding `external`, `environment`, and `proc_macros` fields in addition to edition, source-file limit, and include control. Its default automatically discovers existing Cargo artifacts; add a `ProcMacroDylib` only to override a source crate path and compiled library. Environment values supplied in options take precedence over the rsbundler process environment, affect `CARGO_TARGET_DIR` discovery, and are visible during procedural-macro expansion.

## License

MIT
