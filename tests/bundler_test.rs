use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use rsbundler::{BundleOptions, BundledSource, BundledSourceKind, RustEdition, bundle_file};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct BundleScenario {
    entry: String,
    edition: String,
    external: Vec<String>,
    #[serde(rename = "maxSourceFiles")]
    max_source_files: usize,
    #[serde(rename = "noInlineIncludes")]
    no_inline_includes: bool,
    cfg: Vec<String>,
    #[serde(rename = "useDefaultCfg")]
    use_default_cfg: Option<bool>,
    environment: HashMap<String, String>,
    #[serde(rename = "shouldFail")]
    should_fail: bool,
    #[serde(rename = "errorContains")]
    error_contains: String,
    #[serde(rename = "mustIncludeSources")]
    must_include_sources: Vec<String>,
    #[serde(rename = "mustExcludeSources")]
    must_exclude_sources: Vec<String>,
    #[serde(rename = "mustIncludeFiles")]
    must_include_files: Vec<String>,
    #[serde(rename = "mustExcludeFiles")]
    must_exclude_files: Vec<String>,
    #[serde(rename = "expectedSources")]
    expected_sources: Vec<ExpectedSource>,
    #[serde(rename = "mustContain")]
    must_contain: Vec<String>,
    #[serde(rename = "mustNotContain")]
    must_not_contain: Vec<String>,
    #[serde(rename = "mustContainCount")]
    must_contain_count: HashMap<String, usize>,
    #[serde(rename = "mustAppearInOrder")]
    must_appear_in_order: Vec<String>,
    #[serde(rename = "rustcArgs")]
    rustc_args: Vec<String>,
    #[serde(rename = "rustcEnvironment")]
    rustc_environment: HashMap<String, String>,
    #[serde(rename = "skipOutputCheck")]
    skip_output_check: bool,
}

impl BundleScenario {
    fn with_defaults(mut self) -> Self {
        if self.entry.trim().is_empty() {
            self.entry = "main.rs".to_owned();
        }
        if self.edition.trim().is_empty() {
            self.edition = "2024".to_owned();
        }
        if self.max_source_files == 0 {
            self.max_source_files = 2048;
        }
        self
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpectedSource {
    module_path: String,
    kind: String,
    file: String,
}

#[test]
fn test_bundle_file() {
    let scenario_names = discover_scenario_names();
    let worker_count = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(scenario_names.len());
    let next_scenario = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        let workers = (0..worker_count)
            .map(|_| {
                scope.spawn(|| {
                    loop {
                        let index = next_scenario.fetch_add(1, Ordering::Relaxed);
                        let Some(scenario_name) = scenario_names.get(index) else {
                            return;
                        };
                        let scenario_path = testdata_root().join(scenario_name);
                        let project_root = scenario_project_root(&scenario_path);
                        let scenario = load_scenario(&scenario_path);
                        execute_scenario(scenario_name, &scenario_path, &project_root, &scenario);
                    }
                })
            })
            .collect::<Vec<_>>();

        for worker in workers {
            if let Err(payload) = worker.join() {
                std::panic::resume_unwind(payload);
            }
        }
    });
}

fn execute_scenario(
    scenario_name: &str,
    scenario_path: &Path,
    project_root: &Path,
    scenario: &BundleScenario,
) {
    let result = bundle_file(
        project_root.join(&scenario.entry),
        BundleOptions {
            edition: parse_edition(&scenario.edition, scenario_path),
            max_source_files: scenario.max_source_files,
            inline_includes: !scenario.no_inline_includes,
            external: scenario.external.clone(),
            cfg: scenario.cfg.clone(),
            use_default_cfg: scenario.use_default_cfg.unwrap_or(true),
            environment: scenario
                .environment
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        },
    );

    if scenario.should_fail {
        let error = result.expect_err("expected bundle_file to fail");
        if !scenario.error_contains.is_empty() {
            assert!(
                error.contains(&scenario.error_contains),
                "expected {scenario_name} error to contain {:?}, got: {error}",
                scenario.error_contains
            );
        }
        return;
    }

    let result = result
        .unwrap_or_else(|error| panic!("bundle_file returned error for {scenario_name}: {error}"));
    write_bundled_output(scenario_path, &result.code);
    check_source_expectations(scenario_name, scenario, &result.bundled_source_list);
    check_code_expectations(scenario_name, scenario, &result.code);

    if !scenario.skip_output_check {
        check_runtime_output(scenario_name, project_root, scenario);
    }
}

fn check_source_expectations(
    scenario_name: &str,
    scenario: &BundleScenario,
    sources: &[BundledSource],
) {
    let module_paths = sources
        .iter()
        .map(|source| source.module_path.as_str())
        .collect::<HashSet<_>>();
    let file_names = sources
        .iter()
        .filter_map(|source| Path::new(&source.file_path).file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect::<HashSet<_>>();

    for module_path in &scenario.must_include_sources {
        assert!(
            module_paths.contains(module_path.as_str()),
            "expected {scenario_name} to include source {module_path:?}"
        );
    }
    for module_path in &scenario.must_exclude_sources {
        assert!(
            !module_paths.contains(module_path.as_str()),
            "expected {scenario_name} to exclude source {module_path:?}"
        );
    }
    for file_name in &scenario.must_include_files {
        assert!(
            file_names.contains(file_name),
            "expected {scenario_name} to include file {file_name:?}"
        );
    }
    for file_name in &scenario.must_exclude_files {
        assert!(
            !file_names.contains(file_name),
            "expected {scenario_name} to exclude file {file_name:?}"
        );
    }

    if !scenario.expected_sources.is_empty() {
        let actual = sources
            .iter()
            .map(|source| ExpectedSourceValue {
                module_path: source.module_path.as_str(),
                kind: source_kind_name(source.kind),
                file: Path::new(&source.file_path)
                    .file_name()
                    .expect("bundled source file name")
                    .to_string_lossy()
                    .into_owned(),
            })
            .collect::<Vec<_>>();
        let expected = scenario
            .expected_sources
            .iter()
            .map(|source| ExpectedSourceValue {
                module_path: source.module_path.as_str(),
                kind: source.kind.as_str(),
                file: source.file.clone(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual, expected,
            "source metadata mismatch for {scenario_name}"
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedSourceValue<'a> {
    module_path: &'a str,
    kind: &'a str,
    file: String,
}

fn source_kind_name(kind: BundledSourceKind) -> &'static str {
    match kind {
        BundledSourceKind::Entry => "entry",
        BundledSourceKind::Module => "module",
        BundledSourceKind::Include => "include",
        BundledSourceKind::IncludeStr => "includeStr",
        BundledSourceKind::IncludeBytes => "includeBytes",
    }
}

fn check_code_expectations(scenario_name: &str, scenario: &BundleScenario, code: &str) {
    for token in &scenario.must_contain {
        assert!(
            code.contains(token),
            "expected {scenario_name} bundled code to contain {token:?}"
        );
    }
    for token in &scenario.must_not_contain {
        assert!(
            !code.contains(token),
            "expected {scenario_name} bundled code not to contain {token:?}"
        );
    }
    for (token, expected) in &scenario.must_contain_count {
        assert_eq!(
            code.matches(token).count(),
            *expected,
            "unexpected {token:?} count for {scenario_name}"
        );
    }
    let mut offset = 0;
    for token in &scenario.must_appear_in_order {
        let Some(index) = code[offset..].find(token) else {
            panic!(
                "expected {scenario_name} bundled code to contain {token:?} after byte {offset}"
            );
        };
        offset += index + token.len();
    }
}

fn discover_scenario_names() -> Vec<String> {
    let entries = fs::read_dir(testdata_root()).expect("read testdata directory");
    let mut names = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            Some(name)
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn load_scenario(scenario_path: &Path) -> BundleScenario {
    let case_path = scenario_path.join("case.json");
    let data = fs::read_to_string(&case_path)
        .unwrap_or_else(|error| panic!("read required {}: {error}", case_path.display()));
    serde_json::from_str::<BundleScenario>(&data)
        .unwrap_or_else(|error| panic!("parse {}: {error}", case_path.display()))
        .with_defaults()
}

fn scenario_project_root(scenario_path: &Path) -> PathBuf {
    scenario_path.canonicalize().unwrap_or_else(|error| {
        panic!(
            "resolve scenario path for {}: {error}",
            scenario_path.display()
        )
    })
}

fn write_bundled_output(scenario_path: &Path, code: &str) {
    let output_path = scenario_path.join("bundled.rs");
    if fs::read_to_string(&output_path).is_ok_and(|current| current == code) {
        return;
    }
    fs::write(&output_path, code).unwrap_or_else(|error| {
        panic!(
            "write bundled output for {}: {error}",
            scenario_path.display()
        )
    });
}

fn check_runtime_output(scenario_name: &str, project_root: &Path, scenario: &BundleScenario) {
    let output_root = temporary_test_dir(scenario_name);
    fs::create_dir_all(&output_root).expect("create test output directory");
    let rustc_args = scenario
        .rustc_args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let environment = scenario
        .rustc_environment
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let original = compile_and_run(
        &project_root.join(&scenario.entry),
        &output_root.join(executable_name("original")),
        &scenario.edition,
        &rustc_args,
        &environment,
    );
    let bundled = compile_and_run(
        &project_root.join("bundled.rs"),
        &output_root.join(executable_name("bundled")),
        &scenario.edition,
        &rustc_args,
        &environment,
    );
    assert_eq!(
        original.status.success(),
        bundled.status.success(),
        "runtime status mismatch for {scenario_name}"
    );
    assert!(
        original.status.success(),
        "original {scenario_name} failed:\n{}",
        String::from_utf8_lossy(&original.stderr)
    );
    assert_eq!(
        original.stdout, bundled.stdout,
        "stdout mismatch for {scenario_name}"
    );
}

fn compile_and_run(
    source: &Path,
    executable: &Path,
    edition: &str,
    rustc_args: &[&str],
    environment: &[(&str, &str)],
) -> Output {
    let mut command = Command::new("rustc");
    command
        .arg(format!("--edition={edition}"))
        .args(["--crate-name", "rsbundler_runtime"])
        .args(rustc_args)
        .arg(source)
        .arg("-o")
        .arg(executable);
    for (key, value) in environment {
        command.env(key, value);
    }
    let compile = command
        .output()
        .unwrap_or_else(|error| panic!("run rustc for {}: {error}", source.display()));
    assert!(
        compile.status.success(),
        "compile {}:\n{}",
        source.display(),
        String::from_utf8_lossy(&compile.stderr)
    );
    Command::new(executable)
        .output()
        .unwrap_or_else(|error| panic!("run {}: {error}", executable.display()))
}

fn parse_edition(edition: &str, scenario_path: &Path) -> RustEdition {
    match edition {
        "2015" => RustEdition::Edition2015,
        "2018" => RustEdition::Edition2018,
        "2021" => RustEdition::Edition2021,
        "2024" => RustEdition::Edition2024,
        _ => panic!(
            "unsupported edition {edition:?} in {}",
            scenario_path.join("case.json").display()
        ),
    }
}

fn temporary_test_dir(scenario: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rsbundler-test-{}-{scenario}", std::process::id()))
}

fn executable_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

fn testdata_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}
