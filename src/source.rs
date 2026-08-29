use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(super) struct EntryFile {
    pub(super) canonical: PathBuf,
    pub(super) logical: PathBuf,
}

pub(super) fn resolve_entry_file(path: &Path) -> Result<EntryFile, String> {
    if path.as_os_str().is_empty() {
        return Err("entry file must not be empty".to_owned());
    }
    if path.extension().is_none_or(|extension| extension != "rs") {
        return Err(format!(
            "entry file must have a .rs extension: {}",
            path.display()
        ));
    }
    let logical = path.to_path_buf();
    let canonical = canonical_file(&logical, "entry file")?;
    Ok(EntryFile { canonical, logical })
}

pub(super) fn canonical_file(path: &Path, description: &str) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("resolve {description} {}: {error}", path.display()))?;
    if !canonical.is_file() {
        return Err(format!(
            "{description} is not a file: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

pub(super) fn read_rust_source(path: &Path, keep_shebang: bool) -> Result<String, String> {
    let mut source = fs::read_to_string(path)
        .map_err(|error| format!("read Rust source {}: {error}", path.display()))?;
    if source.starts_with('\u{feff}') {
        source.remove(0);
    }
    if !keep_shebang && source.starts_with("#!") && !source.starts_with("#![") {
        if let Some(newline) = source.find('\n') {
            source.drain(..newline);
        } else {
            source.clear();
        }
    }
    Ok(source)
}

pub(super) fn source_position(path: &Path, source: &str, offset: usize) -> String {
    let prefix = &source[..offset.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or_else(|| prefix.chars().count(), |(_, tail)| tail.chars().count())
        + 1;
    format!("{}:{line}:{column}", path.display())
}

pub(super) fn format_parse_errors(
    path: &Path,
    source: &str,
    errors: &[ra_ap_syntax::SyntaxError],
) -> String {
    let details = errors
        .iter()
        .map(|error| {
            let offset = u32::from(error.range().start()) as usize;
            format!("{}: {error}", source_position(path, source, offset))
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("parse Rust source {}:\n{details}", path.display())
}

pub(super) fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
