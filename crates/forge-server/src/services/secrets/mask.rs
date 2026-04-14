// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Log masking. Every known secret value is replaced with `***` before a log
//! chunk is persisted or broadcast.
//!
//! The mask list is per-run — built from the set of `${{ secrets.<name> }}`
//! references resolved at run start — so a leaked secret from one repo's run
//! can't be silently masked in another.

/// Build a masker for a run. `values` is the list of plaintext secret values
/// referenced by the workflow.
pub struct Mask {
    values: Vec<String>,
}

impl Mask {
    pub fn new(values: Vec<String>) -> Self {
        // Sort by length descending so that if secret A is a prefix of secret
        // B we replace B first (otherwise we'd mask A inside B and leave a
        // trailing suffix of B in the log).
        let mut v = values;
        v.sort_by(|a, b| b.len().cmp(&a.len()));
        v.retain(|s| !s.is_empty());
        Self { values: v }
    }

    /// Clone the raw masked values. Used when spawning per-stream masker
    /// clones without wrapping Mask in Arc (the values list is tiny — a
    /// handful of strings per run).
    pub fn clone_values(&self) -> Vec<String> {
        self.values.clone()
    }

    pub fn apply(&self, s: &str) -> String {
        if self.values.is_empty() {
            return s.to_string();
        }
        let mut out = s.to_string();
        for v in &self.values {
            if out.contains(v) {
                out = out.replace(v, "***");
            }
        }
        out
    }
}
