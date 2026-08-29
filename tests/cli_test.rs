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
