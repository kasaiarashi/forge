// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! `forge trust` — manually pin a forge server's TLS certificate.
//!
//! In most cases you don't need to run this directly: `forge login` does
//! the same trust-on-first-use flow automatically when the server's cert
//! isn't already in your trust store. This command exists as:
//!
//! - an escape hatch for scripts that need to pre-populate trust before
//!   any login attempt (CI bootstrapping, ansible runs, etc.),
//! - a way to re-pin after a server rotates its CA (delete the old file
//!   in `~/.forge/trusted/` and re-run `forge trust`),
//! - an explicit command operators can point users at when the auto
//!   prompt is confusing.
//!
//! It forwards to [`crate::tofu::ensure_trusted`] after resolving any
//! web-UI URL to the underlying gRPC URL via [`crate::url_resolver`], so
//! the pin lands under the key the gRPC client factory will actually look
//! up on subsequent calls.

use anyhow::Result;

pub fn run(server_url: String, yes: bool) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        // Resolve web → gRPC first: "trust this URL" really means "trust
        // the forge-server I'd reach via this URL", which is different
        // from forge-web's own cert.
        let resolved = crate::url_resolver::resolve(&server_url).await;
        crate::tofu::ensure_trusted(&resolved, yes).await
    })
}
