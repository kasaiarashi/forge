// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Random token generation and PAT scope parsing.
//!
//! ## Token format
//!
//! Personal access tokens are 32 bytes of OS randomness rendered as URL-safe
//! base64 (no padding) and prefixed with `fpat_` so users can recognize them
//! and so secret-scanners (`gitleaks`, github push protection, etc.) can match
//! them. Session tokens use the same shape with an `fses_` prefix.
//!
//! The first 12 characters of plaintext (the `fpat_` / `fses_` prefix plus
//! seven random chars) are stored alongside the hash as `token_prefix` so the
//! interceptor can do a fast indexed prefix lookup before paying for an
//! argon2id verify.
//!
//! ## Scopes
//!
//! Scopes are stored as a comma-separated string in SQLite — small enough that
//! a junction table is overkill. The set is closed (we own all the constants)
//! so the parse is exhaustive.

use anyhow::{anyhow, bail, Result};
use rand::RngCore;
use std::fmt;

/// The plaintext + persisted form of a freshly-minted PAT.
///
/// The plaintext is shown to the user **once** at creation time and never
/// again — only [`hash`](Self::hash) is stored in the database.
#[derive(Debug, Clone)]
pub struct PatPlaintext {
    /// The full token to display to the user, e.g. `fpat_abc123…`.
    pub plaintext: String,
    /// The first [`PREFIX_LEN`] chars of the plaintext, indexed in SQLite.
    pub prefix: String,
    /// The argon2id hash of the plaintext, persisted in `token_hash`.
    pub hash: String,
}

/// Length of the indexed `token_prefix` column.
pub const PREFIX_LEN: usize = 12;

/// `fpat_` for personal access tokens.
pub const PAT_PREFIX: &str = "fpat_";
/// `fses_` for browser session tokens.
pub const SESSION_PREFIX: &str = "fses_";

/// Generate a fresh PAT plaintext (`fpat_…`) along with its prefix and hash.
pub fn generate_pat() -> Result<PatPlaintext> {
    generate_with_prefix(PAT_PREFIX)
}

/// Generate a fresh session-token plaintext (`fses_…`) along with its prefix and hash.
pub fn generate_session() -> Result<PatPlaintext> {
    generate_with_prefix(SESSION_PREFIX)
}

fn generate_with_prefix(label: &str) -> Result<PatPlaintext> {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    // URL-safe base64 without padding so the token is shell-safe.
    let body = base64_url_no_pad(&bytes);
    let plaintext = format!("{label}{body}");
    let prefix = plaintext.chars().take(PREFIX_LEN).collect::<String>();
    let hash = super::password::hash(&plaintext)?;
    Ok(PatPlaintext {
        plaintext,
        prefix,
        hash,
    })
}

/// Compute the prefix the database would have stored for a given plaintext
/// token. The interceptor uses this to look up candidate rows.
pub fn prefix_of(plaintext: &str) -> String {
    plaintext.chars().take(PREFIX_LEN).collect()
}

/// Minimal URL-safe base64 without padding. We don't bring in a base64 crate
/// just for this — the alphabet is well known and the body is fixed-size.
fn base64_url_no_pad(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((input.len() * 4 + 2) / 3);
    let mut i = 0;
    while i + 3 <= input.len() {
        let b0 = input[i] as usize;
        let b1 = input[i + 1] as usize;
        let b2 = input[i + 2] as usize;
        out.push(ALPHABET[(b0 >> 2) & 0x3f] as char);
        out.push(ALPHABET[((b0 << 4) | (b1 >> 4)) & 0x3f] as char);
        out.push(ALPHABET[((b1 << 2) | (b2 >> 6)) & 0x3f] as char);
        out.push(ALPHABET[b2 & 0x3f] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let b0 = input[i] as usize;
        out.push(ALPHABET[(b0 >> 2) & 0x3f] as char);
        out.push(ALPHABET[(b0 << 4) & 0x3f] as char);
    } else if rem == 2 {
        let b0 = input[i] as usize;
        let b1 = input[i + 1] as usize;
        out.push(ALPHABET[(b0 >> 2) & 0x3f] as char);
        out.push(ALPHABET[((b0 << 4) | (b1 >> 4)) & 0x3f] as char);
        out.push(ALPHABET[(b1 << 2) & 0x3f] as char);
    }
    out
}

// ── Scopes ───────────────────────────────────────────────────────────────────

/// What a personal access token is allowed to do. Stored on each PAT row at
/// creation time and checked by the per-handler authorization helpers.
///
/// Scopes are *additive*: a token with `RepoWrite` does not implicitly grant
/// `RepoAdmin`. The interceptor stores the parsed set on the [`Caller`] and
/// each handler asks for the specific scope it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Clone, pull, fetch refs, read history. Read-only.
    RepoRead,
    /// Push commits, update refs, acquire/release locks.
    RepoWrite,
    /// Manage repo settings, ACLs, visibility.
    RepoAdmin,
    /// Manage server users, create other admins. Server-wide.
    UserAdmin,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::RepoRead => "repo:read",
            Scope::RepoWrite => "repo:write",
            Scope::RepoAdmin => "repo:admin",
            Scope::UserAdmin => "user:admin",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s.trim() {
            "repo:read" => Ok(Scope::RepoRead),
            "repo:write" => Ok(Scope::RepoWrite),
            "repo:admin" => Ok(Scope::RepoAdmin),
            "user:admin" => Ok(Scope::UserAdmin),
            other => Err(anyhow!("unknown scope '{other}'")),
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Encode a slice of scopes as the canonical comma-separated form for the
/// `personal_access_tokens.scopes` column.
pub fn encode_scopes(scopes: &[Scope]) -> String {
    let mut out = String::new();
    for (i, s) in scopes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(s.as_str());
    }
    out
}

/// Parse the canonical comma-separated form back into a vector of scopes.
/// An empty input string parses as an empty vector — but a PAT row is never
/// allowed to have zero scopes (see [`super::store::SqliteUserStore::create_pat`]).
pub fn parse_scopes(s: &str) -> Result<Vec<Scope>> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for piece in s.split(',') {
        out.push(Scope::parse(piece)?);
    }
    Ok(out)
}

/// Validate that the caller-supplied scope list isn't empty and contains no
/// duplicates. Used by `create_pat` so we don't accept malformed inputs.
pub fn validate_scopes(scopes: &[Scope]) -> Result<()> {
    if scopes.is_empty() {
        bail!("at least one scope is required");
    }
    let mut seen = std::collections::HashSet::new();
    for s in scopes {
        if !seen.insert(*s) {
            bail!("duplicate scope: {s}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pat_format_is_recognizable() {
        let p = generate_pat().unwrap();
        assert!(p.plaintext.starts_with(PAT_PREFIX));
        assert_eq!(p.prefix.len(), PREFIX_LEN);
        assert!(p.plaintext.starts_with(&p.prefix));
        // Verify the hash actually matches the plaintext.
        assert!(super::super::password::verify(&p.plaintext, &p.hash).unwrap());
    }

    #[test]
    fn session_format_is_recognizable() {
        let s = generate_session().unwrap();
        assert!(s.plaintext.starts_with(SESSION_PREFIX));
        assert!(s.prefix.starts_with(SESSION_PREFIX));
    }

    #[test]
    fn pat_uniqueness_across_calls() {
        let a = generate_pat().unwrap();
        let b = generate_pat().unwrap();
        assert_ne!(a.plaintext, b.plaintext);
    }

    #[test]
    fn prefix_of_matches_generated_prefix() {
        let p = generate_pat().unwrap();
        assert_eq!(prefix_of(&p.plaintext), p.prefix);
    }

    #[test]
    fn scope_round_trip() {
        let scopes = vec![Scope::RepoRead, Scope::RepoWrite, Scope::UserAdmin];
        let encoded = encode_scopes(&scopes);
        assert_eq!(encoded, "repo:read,repo:write,user:admin");
        let parsed = parse_scopes(&encoded).unwrap();
        assert_eq!(parsed, scopes);
    }

    #[test]
    fn empty_scopes_parse_as_empty() {
        assert!(parse_scopes("").unwrap().is_empty());
    }

    #[test]
    fn unknown_scope_rejected() {
        assert!(Scope::parse("repo:nuke").is_err());
        assert!(parse_scopes("repo:read,repo:nuke").is_err());
    }

    #[test]
    fn validate_rejects_empty_and_duplicates() {
        assert!(validate_scopes(&[]).is_err());
        assert!(validate_scopes(&[Scope::RepoRead, Scope::RepoRead]).is_err());
        assert!(validate_scopes(&[Scope::RepoRead, Scope::RepoWrite]).is_ok());
    }

    #[test]
    fn base64_url_alphabet_is_url_safe() {
        // Make sure we're not emitting `+` or `/` which would break URL encoding.
        for _ in 0..50 {
            let p = generate_pat().unwrap();
            for c in p.plaintext.chars() {
                assert!(
                    c.is_ascii_alphanumeric() || c == '-' || c == '_',
                    "non-url-safe char in token: {c}"
                );
            }
        }
    }
}
