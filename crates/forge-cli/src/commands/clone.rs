// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use anyhow::{bail, Result};
use forge_core::object::snapshot::Author;
use forge_core::workspace::Workspace;

/// `forge clone <url> [--path <dir>] [--repo <owner/name>]`
///
/// The URL carries the repo path GitHub-style:
///
///     forge clone http://localhost:9876/alice/demo
///
/// gets parsed into:
///
///     server_url = "http://localhost:9876"
///     repo       = "alice/demo"
///     dir        = "./demo"     (last segment, like git)
///
/// Backward compat: if the URL has no `/owner/name` path (e.g.
/// `http://localhost:9876`) then the explicit `--repo` flag is required.
pub fn run(url: String, path: Option<String>, repo: Option<String>) -> Result<()> {
    let (server_url, repo_from_url) = parse_clone_url(&url)?;

    // Resolve the repo identifier. URL path wins over --repo if both present
    // (the URL is the more idiomatic way; --repo is the legacy escape hatch).
    let repo_name = match (repo_from_url, repo) {
        (Some(r), _) => r,
        (None, Some(r)) if !r.is_empty() => r,
        _ => bail!(
            "no repo path in URL — pass it like `forge clone http://host/owner/name` \
             or use --repo <owner/name>"
        ),
    };

    // Derive the local directory name from the LAST segment of the repo path,
    // matching `git clone` behavior. So `forge clone .../alice/demo` clones
    // into `./demo`, not `./alice/demo`.
    let default_dir = repo_name
        .rsplit('/')
        .next()
        .unwrap_or(&repo_name)
        .to_string();
    let dir_name = path.unwrap_or(default_dir);

    let target = std::env::current_dir()?.join(&dir_name);
    if target.exists() && std::fs::read_dir(&target)?.next().is_some() {
        bail!(
            "destination path '{}' already exists and is not empty",
            dir_name
        );
    }
    std::fs::create_dir_all(&target)?;

    println!("Cloning into '{}'...", target.display());

    // Initialize workspace.
    let author = Author {
        name: whoami::fallible::realname().unwrap_or_else(|_| "Unknown".into()),
        email: String::new(),
    };
    let ws = Workspace::init(&target, author)?;

    // Configure remote and repo name (the bare server URL, no /owner/name).
    let mut config = ws.config()?;
    config.add_remote("origin".into(), server_url)?;
    config.repo = repo_name;
    ws.save_config(&config)?;

    // Write default .forgeignore.
    let ignore_path = target.join(".forgeignore");
    if !ignore_path.exists() {
        std::fs::write(&ignore_path, forge_ignore::ForgeIgnore::default_content())?;
    }

    // Pull using the workspace we just created (not cwd).
    super::pull::run_with_workspace(&ws)?;

    println!("Clone complete.");
    Ok(())
}

/// Split a `forge clone` URL into the bare server URL and an optional
/// `<owner>/<name>` repo path. Accepts:
///
///   http://host:port/owner/name      → ("http://host:port", Some("owner/name"))
///   https://host/owner/name/         → ("https://host",      Some("owner/name"))
///   http://host:port                 → ("http://host:port", None)
///   http://host:port/                → ("http://host:port", None)
///
/// Rejects URLs with no scheme or with > 2 path segments.
///
/// Exposed to sibling modules so `forge remote add` can accept the same
/// URL form as `forge clone`.
pub(super) fn parse_clone_url(raw: &str) -> Result<(String, Option<String>)> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("clone URL is empty");
    }
    // Find the scheme separator, then the start of the path.
    let scheme_end = raw
        .find("://")
        .ok_or_else(|| anyhow::anyhow!("URL missing scheme (expected http:// or https://)"))?;
    let after_scheme = &raw[scheme_end + 3..];
    let (host_port, path) = match after_scheme.find('/') {
        Some(idx) => (&after_scheme[..idx], &after_scheme[idx + 1..]),
        None => (after_scheme, ""),
    };
    if host_port.is_empty() {
        bail!("URL missing host");
    }
    let server_url = format!("{}://{}", &raw[..scheme_end], host_port);
    let trimmed_path = path.trim_matches('/');
    if trimmed_path.is_empty() {
        return Ok((server_url, None));
    }
    // Reject deeply-nested paths — `/owner/name` is exactly two segments.
    let segment_count = trimmed_path.split('/').filter(|s| !s.is_empty()).count();
    if segment_count != 2 {
        bail!(
            "expected URL path '<owner>/<name>', got '{}'",
            trimmed_path
        );
    }
    Ok((server_url, Some(trimmed_path.to_string())))
}

#[cfg(test)]
mod tests {
    use super::parse_clone_url;

    #[test]
    fn parses_full_url_with_owner_repo() {
        let (s, r) = parse_clone_url("http://localhost:9876/alice/demo").unwrap();
        assert_eq!(s, "http://localhost:9876");
        assert_eq!(r.as_deref(), Some("alice/demo"));
    }

    #[test]
    fn parses_https_with_trailing_slash() {
        let (s, r) = parse_clone_url("https://forge.acme.com/team/game/").unwrap();
        assert_eq!(s, "https://forge.acme.com");
        assert_eq!(r.as_deref(), Some("team/game"));
    }

    #[test]
    fn parses_bare_server_url() {
        let (s, r) = parse_clone_url("http://localhost:9876").unwrap();
        assert_eq!(s, "http://localhost:9876");
        assert_eq!(r, None);
    }

    #[test]
    fn parses_bare_server_url_trailing_slash() {
        let (s, r) = parse_clone_url("http://localhost:9876/").unwrap();
        assert_eq!(s, "http://localhost:9876");
        assert_eq!(r, None);
    }

    #[test]
    fn rejects_missing_scheme() {
        assert!(parse_clone_url("localhost:9876/alice/demo").is_err());
    }

    #[test]
    fn rejects_too_many_path_segments() {
        assert!(parse_clone_url("http://host/a/b/c").is_err());
    }

    #[test]
    fn rejects_single_path_segment() {
        // /onlyowner is ambiguous — repo is `<owner>/<name>`, not just an owner.
        assert!(parse_clone_url("http://host/onlyowner").is_err());
    }
}
