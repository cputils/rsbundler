use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use toml::Value;

pub(super) struct DiscoveredProcMacro {
    pub(super) crate_names: Vec<String>,
    pub(super) candidates: Vec<PathBuf>,
}

pub(super) fn discover_proc_macros(
    entry_dir: &Path,
    environment: &[(String, String)],
    explicitly_configured: &HashSet<String>,
) -> Vec<DiscoveredProcMacro> {
    let manifests = cargo_manifests(entry_dir);
    if manifests.is_empty() && environment_value(environment, "CARGO_TARGET_DIR").is_none() {
        return Vec::new();
    }
    let aliases = dependency_aliases(&manifests);
    let mut combined = BTreeMap::<Vec<String>, Vec<PathBuf>>::new();
    for target_dir in target_directories(entry_dir, environment, &manifests) {
        for discovered in discover_in_target(&target_dir, &aliases, explicitly_configured) {
            combined
                .entry(discovered.crate_names)
                .or_default()
                .extend(discovered.candidates);
        }
    }
    combined
        .into_iter()
        .map(|(crate_names, candidates)| DiscoveredProcMacro {
            crate_names,
            candidates,
        })
        .collect()
}

fn cargo_manifests(entry_dir: &Path) -> Vec<PathBuf> {
    let mut manifests = Vec::new();
    for directory in entry_dir.ancestors() {
        let manifest = directory.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let is_workspace_root = fs::read_to_string(&manifest)
            .ok()
            .and_then(|source| toml::from_str::<Value>(&source).ok())
            .is_some_and(|document| document.get("workspace").is_some());
        manifests.push(manifest);
        if is_workspace_root {
            break;
        }
    }
    manifests
}

fn target_directories(
    entry_dir: &Path,
    environment: &[(String, String)],
    manifests: &[PathBuf],
) -> Vec<PathBuf> {
    if let Some(path) = environment_value(environment, "CARGO_TARGET_DIR") {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| entry_dir.to_path_buf());
        return vec![resolve_relative(&current_dir, &path)];
    }
    if let Some(path) = configured_target_directory(entry_dir) {
        return vec![path];
    }
    manifests
        .iter()
        .filter_map(|manifest| manifest.parent())
        .map(|directory| directory.join("target"))
        .collect()
}

fn environment_value(environment: &[(String, String)], key: &str) -> Option<String> {
    environment
        .iter()
        .rev()
        .find_map(|(name, value)| (name == key).then(|| value.clone()))
        .or_else(|| std::env::var(key).ok())
}

fn configured_target_directory(entry_dir: &Path) -> Option<PathBuf> {
    for directory in entry_dir.ancestors() {
        for name in ["config.toml", "config"] {
            let config = directory.join(".cargo").join(name);
            let Ok(source) = fs::read_to_string(&config) else {
                continue;
            };
            let Ok(document) = toml::from_str::<Value>(&source) else {
                continue;
            };
            let Some(path) = document
                .get("build")
                .and_then(Value::as_table)
                .and_then(|build| build.get("target-dir"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            return Some(resolve_relative(config.parent().unwrap_or(directory), path));
        }
    }
    None
}

fn resolve_relative(base: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn dependency_aliases(manifests: &[PathBuf]) -> HashMap<String, Vec<String>> {
    let mut aliases = HashMap::<String, Vec<String>>::new();
    for manifest in manifests {
        let Some(document) = fs::read_to_string(manifest)
            .ok()
            .and_then(|source| toml::from_str::<Value>(&source).ok())
        else {
            continue;
        };
        collect_dependency_tables(&document, &mut aliases);
        if let Some(targets) = document.get("target").and_then(Value::as_table) {
            for target in targets.values() {
                collect_dependency_tables(target, &mut aliases);
            }
        }
        if let Some(workspace) = document.get("workspace") {
            collect_dependency_tables(workspace, &mut aliases);
        }
    }
    for crate_names in aliases.values_mut() {
        crate_names.sort();
        crate_names.dedup();
    }
    aliases
}

fn collect_dependency_tables(document: &Value, aliases: &mut HashMap<String, Vec<String>>) {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(dependencies) = document.get(section).and_then(Value::as_table) else {
            continue;
        };
        for (alias, specification) in dependencies {
            let package = specification
                .as_table()
                .and_then(|table| table.get("package"))
                .and_then(Value::as_str)
                .unwrap_or(alias);
            aliases
                .entry(normalize_name(package))
                .or_default()
                .push(normalize_name(alias));
        }
    }
}

fn discover_in_target(
    target_dir: &Path,
    aliases: &HashMap<String, Vec<String>>,
    explicitly_configured: &HashSet<String>,
) -> Vec<DiscoveredProcMacro> {
    let mut artifacts = BTreeMap::<String, Vec<PathBuf>>::new();
    for deps_dir in dependency_directories(target_dir) {
        let Ok(entries) = fs::read_dir(deps_dir) else {
            continue;
        };
        let mut paths = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .filter_map(|path| artifact_crate_name(&path).map(|name| (name, path)))
            .collect::<Vec<_>>();
        paths.sort_by_key(|(_, path)| Reverse(modified(path)));
        for (crate_name, path) in paths {
            artifacts.entry(crate_name).or_default().push(path);
        }
    }

    artifacts
        .into_iter()
        .filter_map(|(artifact_name, candidates)| {
            let crate_names = aliases
                .get(&artifact_name)
                .cloned()
                .unwrap_or_else(|| vec![artifact_name]);
            crate_names
                .iter()
                .all(|name| !explicitly_configured.contains(name))
                .then_some(DiscoveredProcMacro {
                    crate_names,
                    candidates,
                })
        })
        .collect()
}

fn dependency_directories(target_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(target_dir) else {
        return Vec::new();
    };
    let mut first_level = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    first_level.sort_by_key(|path| profile_priority(path));

    let mut result = first_level
        .iter()
        .map(|directory| directory.join("deps"))
        .filter(|directory| directory.is_dir())
        .collect::<Vec<_>>();
    for target in first_level {
        let Ok(entries) = fs::read_dir(target) else {
            continue;
        };
        let mut profiles = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        profiles.sort_by_key(|path| profile_priority(path));
        result.extend(
            profiles
                .into_iter()
                .map(|profile| profile.join("deps"))
                .filter(|directory| directory.is_dir()),
        );
    }
    result
}

fn profile_priority(path: &Path) -> (u8, String) {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let priority = match name.as_str() {
        "debug" => 0,
        "release" => 1,
        _ => 2,
    };
    (priority, name)
}

fn artifact_crate_name(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let stem = file_name
        .strip_prefix(std::env::consts::DLL_PREFIX)?
        .strip_suffix(std::env::consts::DLL_SUFFIX)?;
    let crate_name = stem
        .rsplit_once('-')
        .filter(|(_, hash)| hash.len() >= 8 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map_or(stem, |(name, _)| name);
    (!crate_name.is_empty()).then(|| normalize_name(crate_name))
}

fn normalize_name(name: &str) -> String {
    name.trim_start_matches("r#").replace('-', "_")
}

fn modified(path: &Path) -> SystemTime {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}
