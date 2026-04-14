//! Per-file diff record — what the formatters consume.

/// One file's worth of diff input: path, status, binary flag, and raw content
/// for both sides. `old_content`/`new_content` are empty for adds/deletes.
pub struct FileDiff {
    pub path: String,
    pub status: &'static str,
    pub binary: bool,
    pub old_content: Vec<u8>,
    pub new_content: Vec<u8>,
}
