use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn bundles_to_standard_output_without_external_tools() {
    let entry = testdata_root().join("basic/main.rs");
    let output = Command::new(env!("CARGO_BIN_EXE_rsbundler"))
        .arg(entry)
        .env("PATH", "")
        .output()
        .expect("run rsbundler");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("CLI output should be UTF-8");
    assert!(stdout.contains("mod math {"));
    assert!(stdout.contains("pub mod nested {"));
}

#[test]
fn bundles_to_output_file() {
    let output_path = std::env::temp_dir().join(format!(
        "rsbundler-cli-test-{}-bundled.rs",
        std::process::id()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_rsbundler"))
        .arg(testdata_root().join("basic/main.rs"))
        .args(["--output"])
        .arg(&output_path)
        .output()
        .expect("run rsbundler");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let bundled = fs::read_to_string(&output_path).expect("read bundled output");
    fs::remove_file(output_path).expect("remove bundled output");
    assert!(bundled.contains("mod data {"));
}

#[test]
fn discovers_proc_macros_without_an_external_server() {
    let fixture = testdata_root().join("proc-macro-support");
    let target_dir = std::env::temp_dir().join(format!(
        "rsbundler-cli-proc-macro-target-{}",
        std::process::id()
    ));
    let deps_dir = target_dir.join("debug/deps");
    fs::create_dir_all(&deps_dir).expect("create Cargo artifact directory");
    let dylib_path = deps_dir.join(format!(
        "{}fixture_macro_implementation-deadbeef{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    ));
    let compile = Command::new("rustc")
        .args(["--edition=2024", "--crate-type=proc-macro", "--crate-name"])
        .arg("fixture_macro_implementation")
        .arg(fixture.join("macros.rs"))
        .arg("-o")
        .arg(&dylib_path)
        .output()
        .expect("compile proc-macro fixture");
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let output = Command::new(env!("CARGO_BIN_EXE_rsbundler"))
        .arg(fixture.join("main.rs"))
        .env("CARGO_TARGET_DIR", &target_dir)
        .env("PATH", "")
        .output()
        .expect("run rsbundler with automatic proc-macro discovery");
    fs::remove_dir_all(target_dir).expect("remove proc-macro target directory");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("CLI output should be UTF-8");
    assert!(
        stdout.contains("mod dependency {"),
        "procedural macro was not expanded:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("impl Marker"));
    assert!(!stdout.contains("fixture_macros::"));
}

#[test]
fn reports_bundling_errors() {
    let output = Command::new(env!("CARGO_BIN_EXE_rsbundler"))
        .arg("missing.rs")
        .output()
        .expect("run rsbundler");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("CLI error should be UTF-8");
    assert!(stderr.starts_with("rsbundler: "));
    assert!(stderr.contains("entry file"));
}

fn testdata_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}
