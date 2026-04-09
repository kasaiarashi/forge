//! Pager helper for long-form CLI output (like `git log` piping to `less`).
//!
//! Routes content through the cross-platform `minus` pager when stdout is a
//! terminal and the caller hasn't opted out, otherwise writes directly.

use std::io::{IsTerminal, Write};

/// Display `content` to the user. If stdout is a TTY and `no_pager`/`json` are
/// false, hands off to the `minus` pager (supports scrolling, search, `q`/`:q`
/// to exit). Otherwise writes straight to stdout.
///
/// Falls back to direct stdout if the pager fails to start, so the caller never
/// loses output.
pub fn show(content: String, no_pager: bool, json: bool) {
    let use_pager = !no_pager && !json && std::io::stdout().is_terminal();

    if !use_pager {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(content.as_bytes());
        return;
    }

    // Try paging via minus. If it errors at any step, fall back to direct print
    // so the user always sees their output.
    let pager = minus::Pager::new();
    if pager.set_text(content.clone()).is_ok() && minus::page_all(pager).is_ok() {
        return;
    }

    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(content.as_bytes());
}
