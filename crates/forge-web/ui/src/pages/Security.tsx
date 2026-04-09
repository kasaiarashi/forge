// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

import LegalLayout from '../components/LegalLayout';

/**
 * `/security` — high-level security model + how to report vulnerabilities.
 * Reflects the post-audit hardening: argon2id, scope-gated PATs, per-handler
 * authz, TLS-by-default with TOFU pinning, opt-in workflow engine, etc.
 */
export default function SecurityPage() {
  return (
    <LegalLayout
      title="Security"
      subtitle="How Forge VCS protects your code, credentials, and your server."
    >
      <p>
        Forge VCS treats security as a deployment-grade concern, not a
        feature flag. The defaults are designed for production: TLS on,
        passwords hashed with argon2id, every gRPC handler runs an explicit
        authorization check, and the workflow engine — which executes
        arbitrary shell commands — is opt-in and off by default.
      </p>

      <Section title="Authentication">
        <ul>
          <li>
            <strong>argon2id</strong> for both passwords and bearer tokens
            (PATs and session tokens), with random per-secret salts.
          </li>
          <li>
            <strong>32 bytes of OS entropy</strong> per token, prefixed{' '}
            <code>fpat_</code> (PATs) or <code>fses_</code> (sessions) so
            secret scanners can match them.
          </li>
          <li>
            <strong>Constant-time comparison</strong> on the bootstrap-token
            check used to gate first-admin creation.
          </li>
          <li>
            <strong>One-time bootstrap token</strong> printed to the server
            log on first start, consumed after the first admin is created
            so a publicly-reachable fresh install can't be hijacked by a
            stranger.
          </li>
        </ul>
      </Section>

      <Section title="Authorization">
        <ul>
          <li>
            <strong>Per-handler check</strong> on every gRPC method:{' '}
            <code>require_authenticated</code>,{' '}
            <code>require_repo_read</code>, <code>require_repo_write</code>,{' '}
            <code>require_repo_admin</code>, <code>require_server_admin</code>.
          </li>
          <li>
            <strong>Scope-gated PATs:</strong> a token created with{' '}
            <code>repo:read</code> can clone but not push, regardless of
            the underlying user's role on a repo.
          </li>
          <li>
            <strong>Namespace enforcement:</strong> non-admin users can
            only create repos in their own namespace
            (<code>&lt;username&gt;/&lt;repo&gt;</code>); cross-namespace
            creation is rejected with 403.
          </li>
        </ul>
      </Section>

      <Section title="Transport">
        <ul>
          <li>
            <strong>TLS by default.</strong> Both forge-server (gRPC) and
            forge-web auto-generate a local CA + leaf certificate on first
            start using <code>rcgen</code>, sign the leaf to cover every
            non-loopback interface IP, and serve HTTPS without any operator
            cert handling.
          </li>
          <li>
            <strong>SSH-style trust on first use.</strong> The CLI prints
            the CA fingerprint on the first connect and asks the user to
            confirm; subsequent connections use the pinned cert
            automatically. The same machinery accepts publicly-trusted
            certs (Let's Encrypt) without unnecessarily pinning.
          </li>
          <li>
            <strong>Web → gRPC auto-resolve.</strong> If the user pastes
            the web URL into <code>forge login</code>, the CLI probes the
            server's well-known endpoint and transparently switches to the
            underlying gRPC URL.
          </li>
        </ul>
      </Section>

      <Section title="Web hardening">
        <ul>
          <li>
            <strong>Strict CORS:</strong> empty allowed-origins means no
            cross-origin requests at all (the SPA is same-origin). Wildcard
            reflection with credentials is never enabled.
          </li>
          <li>
            <strong>Security headers</strong> on every response:
            Content-Security-Policy, X-Frame-Options DENY,
            X-Content-Type-Options nosniff, Referrer-Policy no-referrer,
            HSTS when TLS is on.
          </li>
          <li>
            <strong>Rate limiting</strong> on <code>/api/auth/*</code>{' '}
            (1 req/s sustained, burst 5 by default). Hot paths like push
            and pull are unlimited.
          </li>
          <li>
            <strong>Sanitized error messages.</strong> The web layer never
            forwards <code>tonic::Code::Internal</code> messages to the
            client, so internal SQL/IO error strings can't leak.
          </li>
          <li>
            <strong>RFC 5987 filename encoding</strong> on raw downloads to
            prevent header injection via crafted filenames.
          </li>
        </ul>
      </Section>

      <Section title="Workflow engine">
        <p>
          Forge Actions executes shell commands on the server host as the
          forge-server process user. That's a meaningful piece of
          authority: anyone with <code>repo:admin</code> on any repo can
          author a workflow that runs arbitrary code. For that reason, the
          engine is <strong>off by default</strong>. Operators must
          explicitly opt in via <code>actions.enabled = true</code> in{' '}
          <code>forge-server.toml</code> and should run forge-server in an
          isolated environment (container, dedicated unprivileged user,
          dedicated VM) before doing so.
        </p>
      </Section>

      <Section title="Reporting a vulnerability">
        <p>
          If you've found a security issue in the upstream Forge VCS
          source code, please report it privately via the{' '}
          <a
            href="https://github.com/anthropics/forge-vcs/security/advisories/new"
            target="_blank"
            rel="noopener noreferrer"
          >
            GitHub Security Advisories
          </a>{' '}
          flow. For issues specific to <em>this</em> Forge instance (a
          configuration bug, a leaked credential, etc.), see the{' '}
          <a href="/contact">Contact page</a> for the operator's address.
        </p>
        <p>
          Please don't open public issues for security reports until a fix
          has been released.
        </p>
      </Section>
    </LegalLayout>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section style={{ marginTop: '32px' }}>
      <h2
        style={{
          fontSize: '20px',
          fontWeight: 600,
          color: 'var(--fg-default)',
          margin: '0 0 12px 0',
          paddingBottom: '8px',
          borderBottom: '1px solid var(--border-muted)',
        }}
      >
        {title}
      </h2>
      {children}
    </section>
  );
}
