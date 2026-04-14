//! `forge diff` command — thin orchestrator.
//!
//! Arg validation + dispatch only. Blob reading lives in [`blob`], source
//! selection (working dir vs index vs commit) in [`source`], and all output
//! formatting in `forge_diff::format`.

mod blob;
mod source;

use anyhow::{bail, Result};

use crate::pager;
use forge_core::index::Index;
use forge_core::workspace::Workspace;
use forge_diff::format::{colored, extract, json, stat};

pub fn run(
    commit: Option<String>,
    staged: bool,
    stat_flag: bool,
    extract_flag: bool,
    paths: Vec<String>,
    no_pager: bool,
    json_flag: bool,
    class_stats: bool,
) -> Result<()> {
    if staged && commit.is_some() {
        bail!("Cannot use --staged with --commit");
    }

    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover(&cwd)?;
    let index = Index::load(&ws.forge_dir().join("index"))?;

    let filter: Vec<String> = paths
        .iter()
        .map(|p| p.replace('\\', "/").trim_start_matches("./").to_string())
        .collect();

    let file_diffs = if let Some(ref commit_str) = commit {
        source::diff_commit(&ws, commit_str, &filter)?
    } else if staged {
        source::diff_staged(&ws, &index, &filter)?
    } else {
        source::diff_unstaged(&ws, &index, &filter)?
    };

    // --extract writes temp files and prints their paths; not useful to page.
    if extract_flag {
        extract::print_extract(&file_diffs)?;
        return Ok(());
    }

    let mut buffer = String::new();
    if json_flag {
        json::format_json(&file_diffs, &mut buffer)?;
    } else if stat_flag {
        stat::format_stat(&file_diffs, &mut buffer);
    } else {
        colored::format_colored(&file_diffs, &mut buffer, class_stats);
    }

    pager::show(buffer, no_pager, json_flag);

    Ok(())
}
