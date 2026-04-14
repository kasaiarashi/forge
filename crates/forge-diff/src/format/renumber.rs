//! UE auto-renumber collapse for export add/remove pairs.
//!
//! When UE re-saves a Blueprint, it frequently renumbers internal
//! auto-generated exports like `InpActEvt_IA_Look_K2Node_EnhancedInputActionEvent_3`
//! → `_12`, and `EnhancedInputActionValueBinding_3` → `_1`. The logical object
//! is the same; only the suffix integer changed. Treating each rename as a
//! remove+add drowns real user changes in noise.
//!
//! Heuristic: pair each (remove, add) that shares the same logical-prefix
//! (name with trailing `_<digits>` stripped) and the same class. Matched pairs
//! are removed from both vectors. Only applies to classes in
//! [`is_auto_renumbered_class`] — user-authored K2Node add/remove events MUST
//! still surface and are never collapsed.

use std::collections::HashMap;

/// Strip a trailing `_<digits>` suffix. Returns the logical prefix, or `None`
/// if the name doesn't end with a numeric suffix.
pub fn strip_numeric_suffix(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == bytes.len() || i == 0 || bytes[i - 1] != b'_' {
        return None;
    }
    Some(&name[..i - 1])
}

/// Classes whose name-suffix integer UE routinely renumbers on save. We
/// deliberately do NOT list `K2Node_*` here — user-authored graph node
/// add/remove events MUST still surface.
pub fn is_auto_renumbered_class(class: &str) -> bool {
    class == "Function"
        || class.ends_with("DelegateBinding")
        || class.ends_with("ActionValueBinding")
        || class.ends_with("KeyDelegateBinding")
}

/// Find and remove matched (remove, add) pairs that differ only in their
/// trailing numeric suffix and share a class. Returns the number of pairs
/// collapsed, so the caller can surface a summary line.
pub fn collapse_renumber_pairs(
    adds: &mut Vec<(String, String)>,
    removes: &mut Vec<(String, String)>,
) -> usize {
    let mut remove_idx: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, (name, class)) in removes.iter().enumerate() {
        if !is_auto_renumbered_class(class) {
            continue;
        }
        if let Some(prefix) = strip_numeric_suffix(name) {
            remove_idx
                .entry((prefix.to_string(), class.clone()))
                .or_default()
                .push(i);
        }
    }

    let mut consumed_add: Vec<bool> = vec![false; adds.len()];
    let mut consumed_remove: Vec<bool> = vec![false; removes.len()];
    let mut pairs = 0usize;

    for (i, (name, class)) in adds.iter().enumerate() {
        if !is_auto_renumbered_class(class) {
            continue;
        }
        let Some(prefix) = strip_numeric_suffix(name) else { continue };
        let key = (prefix.to_string(), class.clone());
        let Some(idxs) = remove_idx.get_mut(&key) else { continue };
        while let Some(r) = idxs.pop() {
            if !consumed_remove[r] {
                consumed_remove[r] = true;
                consumed_add[i] = true;
                pairs += 1;
                break;
            }
        }
    }

    for i in (0..adds.len()).rev() {
        if consumed_add[i] {
            adds.swap_remove(i);
        }
    }
    for i in (0..removes.len()).rev() {
        if consumed_remove[i] {
            removes.swap_remove(i);
        }
    }

    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_suffix_basic() {
        assert_eq!(strip_numeric_suffix("Foo_3"), Some("Foo"));
        assert_eq!(strip_numeric_suffix("Foo_K2Node_Bar_42"), Some("Foo_K2Node_Bar"));
        assert_eq!(strip_numeric_suffix("NoSuffix"), None);
        assert_eq!(strip_numeric_suffix("TrailingUnderscore_"), None);
        assert_eq!(strip_numeric_suffix(""), None);
    }

    #[test]
    fn collapses_matched_pairs() {
        let mut adds = vec![
            ("InpActEvt_IA_Look_K2Node_EnhancedInputActionEvent_12".into(), "Function".into()),
            ("EnhancedInputActionValueBinding_1".into(), "EnhancedInputActionValueBinding".into()),
            ("MyUserNode_K2Node_CallFunction_99".into(), "K2Node_CallFunction".into()),
        ];
        let mut removes = vec![
            ("InpActEvt_IA_Look_K2Node_EnhancedInputActionEvent_3".into(), "Function".into()),
            ("EnhancedInputActionValueBinding_3".into(), "EnhancedInputActionValueBinding".into()),
        ];
        let n = collapse_renumber_pairs(&mut adds, &mut removes);
        assert_eq!(n, 2);
        assert_eq!(adds.len(), 1);
        assert_eq!(adds[0].0, "MyUserNode_K2Node_CallFunction_99");
        assert!(removes.is_empty());
    }

    #[test]
    fn leaves_unpaired_adds_and_removes() {
        let mut adds = vec![
            ("NewEvent_Function_5".into(), "Function".into()),
        ];
        let mut removes = vec![
            ("OldEvent_Function_2".into(), "Function".into()),
        ];
        let n = collapse_renumber_pairs(&mut adds, &mut removes);
        assert_eq!(n, 0);
        assert_eq!(adds.len(), 1);
        assert_eq!(removes.len(), 1);
    }

    #[test]
    fn does_not_collapse_user_classes() {
        let mut adds = vec![
            ("MyNode_K2Node_CallFunction_5".into(), "K2Node_CallFunction".into()),
        ];
        let mut removes = vec![
            ("MyNode_K2Node_CallFunction_3".into(), "K2Node_CallFunction".into()),
        ];
        let n = collapse_renumber_pairs(&mut adds, &mut removes);
        assert_eq!(n, 0);
        assert_eq!(adds.len(), 1);
        assert_eq!(removes.len(), 1);
    }
}
