// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

use anyhow::{anyhow, bail, Context, Result};
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
///
/// ## Resumable clone
///
/// Game projects can be hundreds of gigabytes; a failed pull halfway
/// through a fresh clone used to leave an orphan `.forge/` directory that
/// blocked any retry. Now the command **resumes** instead:
///
///   * If the target directory is empty → init a new workspace + pull.
///   * If the target already has `.forge/` AND it points at the same
///     `server_url` + `repo_name` we're cloning → skip init, run pull
///     against the existing workspace. Any objects already downloaded are
///     kept, and `pull` fetches only what's still missing.
///   * If the target has `.forge/` pointing at a *different* remote →
///     abort with a clear diagnostic so we don't silently corrupt an
///     unrelated workspace.
///   * If the target has non-forge files → abort as before.
///
/// The pull step itself is NOT wrapped in cleanup. Partial downloads stay
/// on disk so the next `forge clone <same-url>` picks up exactly where
/// the previous run failed.
pub fn run(url: String, path: Option<String>, repo: Option<String>) -> Result<()> {
    let (server_url, repo_from_url) = parse_clone_url(&url)?;

    // Stash the server URL so if the clone-time pull fails with an auth
    // error, the pretty-error handler's "Login now?" prompt pre-fills the
    // URL instead of asking the user to re-type what they already passed.
    crate::set_server_url_hint(&server_url);

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

    // Decide between fresh clone and resume based on what's already at
    // the target.
    let ws = match resolve_target(&target, &server_url, &repo_name)? {
        TargetState::FreshOrEmpty => {
            std::fs::create_dir_all(&target)?;
            println!("Cloning into '{}'...", target.display());
            init_workspace(&target, &server_url, &repo_name)?
        }
        TargetState::ResumeExisting(ws) => {
            println!(
                "Resuming clone into '{}' (existing workspace, remote matches)...",
                target.display()
            );
            ws
        }
    };

    // Pull using the workspace we just created/resumed (not cwd). Any
    // failure here is left on disk — next `forge clone <same-url>` will
    // take the ResumeExisting path and finish what we started.
    super::pull::run_with_workspace(&ws)?;

    println!("Clone complete.");
    Ok(())
}

/// What the target directory currently looks like.
enum TargetState {
    /// The directory doesn't exist, or exists but is empty. Caller should
    /// create it (if needed) and initialize a fresh workspace.
    FreshOrEmpty,
    /// The directory already holds a forge workspace whose remote and
    /// repo match the ones we were asked to clone. Caller should skip
    /// the init step and run pull against the returned workspace.
    ResumeExisting(Workspace),
}

fn resolve_target(
    target: &std::path::Path,
    server_url: &str,
    repo_name: &str,
) -> Result<TargetState> {
    if !target.exists() {
        return Ok(TargetState::FreshOrEmpty);
    }

    let forge_dir = target.join(".forge");
    if forge_dir.is_dir() {
        // Existing workspace. Resume only if the remote lines up with
        // what we were asked to clone — never silently adopt an unrelated
        // workspace under the same path.
        let ws = Workspace::discover(target)
            .with_context(|| format!("opening existing workspace at {}", target.display()))?;
        let cfg = ws
            .config()
            .with_context(|| format!("reading config for {}", target.display()))?;

        let existing_remote = cfg.default_remote_url().ok_or_else(|| {
            anyhow!(
                "existing .forge at {} has no remote configured — delete it \
                     or use a different target path",
                target.display()
            )
        })?;

        if existing_remote != server_url || cfg.repo != repo_name {
            bail!(
                "destination path '{}' is a different forge workspace \
                 (existing remote: {} / repo: {}; requested: {} / {}). \
                 Delete it or choose a different target to start a fresh clone.",
                target.display(),
                existing_remote,
                cfg.repo,
                server_url,
                repo_name
            );
        }

        return Ok(TargetState::ResumeExisting(ws));
    }

    // Directory exists but isn't a forge workspace. Only proceed if it's
    // completely empty — we don't want to scribble on top of whatever
    // the user has there.
    if std::fs::read_dir(target)?.next().is_some() {
        bail!(
            "destination path '{}' already exists and is not empty",
            target.display()
        );
    }
    Ok(TargetState::FreshOrEmpty)
}

/// Create a fresh workspace at `target` and seed its config with the
/// clone URL + repo name. Extracted so the resume path doesn't duplicate
/// it.
fn init_workspace(
    target: &std::path::Path,
    server_url: &str,
    repo_name: &str,
) -> Result<Workspace> {
    let author = Author {
        name: whoami::fallible::realname().unwrap_or_else(|_| "Unknown".into()),
        email: String::new(),
    };
    let ws = Workspace::init(target, author)?;

    let mut config = ws.config()?;
    config.add_remote("origin".into(), server_url.to_string())?;
    config.repo = repo_name.to_string();
    ws.save_config(&config)?;

    // Write default .forgeignore (only if the template didn't already
    // leave one).
    let ignore_path = target.join(".forgeignore");
    if !ignore_path.exists() {
        std::fs::write(&ignore_path, forge_ignore::ForgeIgnore::default_content())?;
    }

    Ok(ws)
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
        bail!("expected URL path '<owner>/<name>', got '{}'", trimmed_path);
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
