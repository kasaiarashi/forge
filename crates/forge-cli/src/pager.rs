//! Pager helper for long-form CLI output.
//!
//! When content exceeds the terminal height, prints it to stdout first (for
//! scrollback persistence) then opens the minus pager on the alternate screen.
//! Since minus calls process::exit on quit, the pre-printed content is what
//! the user sees after exiting. When content fits on screen, prints directly
//! without invoking the pager.

use std::io::{IsTerminal, Write};

pub fn show(content: String, no_pager: bool, json: bool) {
    let use_pager = !no_pager && !json && std::io::stdout().is_terminal();

    if !use_pager {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(content.as_bytes());
        return;
    }

    // Check if content fits on screen — if so, just print directly.
    let line_count = content.lines().count();
    let term_rows = crossterm::terminal::size()
        .map(|(_, rows)| rows as usize)
        .unwrap_or(24);

    if line_count < term_rows {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(content.as_bytes());
        if !content.ends_with('\n') {
            let _ = stdout.write_all(b"\n");
        }
        return;
    }

    // Content is longer than the terminal — use the pager.
    // Print to stdout first so it lands in scrollback. The pager's alternate
    // screen will hide it while active, and when minus exits (via
    // process::exit), the main screen with the content is restored.
    {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(content.as_bytes());
        if !content.ends_with('\n') {
            let _ = stdout.write_all(b"\n");
        }
        let _ = stdout.flush();
    }

    let pager = minus::Pager::new();
    if pager.set_text(content).is_ok() {
        let _ = minus::page_all(pager);
    }
}
