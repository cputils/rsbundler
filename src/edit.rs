use std::path::Path;

#[derive(Debug)]
pub(super) struct Edit {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) replacement: String,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ByteRange {
    pub(super) start: usize,
    pub(super) end: usize,
}

pub(super) fn apply_edits(
    source: &str,
    mut edits: Vec<Edit>,
    file_path: &Path,
) -> Result<String, String> {
    edits.sort_by_key(|edit| (edit.start, edit.end));
    for pair in edits.windows(2) {
        if pair[0].end > pair[1].start {
            return Err(format!(
                "internal error: overlapping replacements while bundling {}",
                file_path.display()
            ));
        }
    }
    let mut output = source.to_owned();
    for edit in edits.into_iter().rev() {
        if edit.end > output.len()
            || edit.start > edit.end
            || !output.is_char_boundary(edit.start)
            || !output.is_char_boundary(edit.end)
        {
            return Err(format!(
                "internal error: invalid replacement range while bundling {}",
                file_path.display()
            ));
        }
        output.replace_range(edit.start..edit.end, &edit.replacement);
    }
    Ok(output)
}

pub(super) fn overlaps_any(edits: &[Edit], range: &ByteRange) -> bool {
    edits
        .iter()
        .any(|edit| edit.start < range.end && range.start < edit.end)
}

pub(super) fn byte_range(range: ra_ap_syntax::TextRange) -> ByteRange {
    ByteRange {
        start: u32::from(range.start()) as usize,
        end: u32::from(range.end()) as usize,
    }
}
