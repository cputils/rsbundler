use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use rsbundler::{BundleOptions, ProcMacroDylib, RustEdition, bundle_file};

/// Bundle a Rust crate's local source dependencies into one source file.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// Rust crate root to bundle, usually src/main.rs or src/lib.rs.
    entry: PathBuf,

    /// Write the bundle to this file instead of standard output.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Rust edition used while parsing the source.
    #[arg(long, value_enum, default_value_t = CliEdition::Edition2024)]
    edition: CliEdition,

    /// Maximum number of source-file dependencies to expand.
    #[arg(long, default_value_t = 2048)]
    max_source_files: usize,

    /// Keep a top-level module out-of-line; repeatable. `// bundle` overrides it.
    #[arg(short, long)]
    external: Vec<String>,

    /// Set a compile-time environment value used by env! in include paths.
    #[arg(long, value_name = "KEY=VALUE")]
    env: Vec<String>,

    /// Override an automatically discovered procedural macro with CRATE=DYLIB.
    #[arg(long, value_name = "CRATE=DYLIB")]
    proc_macro: Vec<String>,

    /// Leave include!, include_str!, and include_bytes! unchanged.
    #[arg(long)]
    no_inline_includes: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliEdition {
    #[value(name = "2015")]
    Edition2015,
    #[value(name = "2018")]
    Edition2018,
    #[value(name = "2021")]
    Edition2021,
    #[value(name = "2024")]
    Edition2024,
}

impl From<CliEdition> for RustEdition {
    fn from(value: CliEdition) -> Self {
        match value {
            CliEdition::Edition2015 => Self::Edition2015,
            CliEdition::Edition2018 => Self::Edition2018,
            CliEdition::Edition2021 => Self::Edition2021,
            CliEdition::Edition2024 => Self::Edition2024,
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rsbundler: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let environment = cli
        .env
        .iter()
        .map(|value| {
            let (key, value) = value
                .split_once('=')
                .ok_or_else(|| format!("--env requires KEY=VALUE, found {value:?}"))?;
            if key.is_empty() {
                return Err("--env key must not be empty".to_owned());
            }
            Ok((key.to_owned(), value.to_owned()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let proc_macros = cli
        .proc_macro
        .iter()
        .map(|value| {
            let (crate_name, dylib_path) = value
                .split_once('=')
                .ok_or_else(|| format!("--proc-macro requires CRATE=DYLIB, found {value:?}"))?;
            if crate_name.is_empty() || dylib_path.is_empty() {
                return Err(format!(
                    "--proc-macro requires non-empty CRATE and DYLIB, found {value:?}"
                ));
            }
            Ok(ProcMacroDylib {
                crate_name: crate_name.to_owned(),
                dylib_path: dylib_path.into(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let result = bundle_file(
        &cli.entry,
        BundleOptions {
            edition: cli.edition.into(),
            max_source_files: cli.max_source_files,
            inline_includes: !cli.no_inline_includes,
            external: cli.external,
            environment,
            proc_macros,
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
