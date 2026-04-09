// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

import LegalLayout from '../components/LegalLayout';

/**
 * `/privacy` — what data the *software* collects and stores. Self-hosted
 * means there is no upstream telemetry, no analytics SDKs, no third-party
 * cookies. The list below is everything that ends up on disk inside
 * `<base_path>` and what each piece is used for.
 */
export default function PrivacyPage() {
  return (
    <LegalLayout
      title="Privacy"
      subtitle="What this Forge instance stores about you, and what it does not."
    >
      <p>
        Forge VCS is self-hosted. It does not phone home, send telemetry to
        the upstream maintainers, or load any third-party analytics SDKs in
        the browser. Everything below lives only on the server you're
        currently authenticated against; nothing is shared with anyone else
        unless your operator has set up custom federation.
      </p>

      <Section title="What we store">
        <ul>
          <li>
            <strong>Account data:</strong> username, email address, display
            name, and an argon2id hash of your password. Plain-text
            passwords are never persisted.
          </li>
          <li>
            <strong>Personal access tokens (PATs):</strong> the random 32-byte
            secret is hashed with argon2id; only an indexed 12-character
            prefix is stored alongside for fast lookup. Plaintext PATs are
            shown to you exactly once at creation time.
          </li>
          <li>
            <strong>Sessions:</strong> when you log in via the web UI, a
            short-lived session token is stored as an HttpOnly, Secure,
            SameSite=Strict cookie in your browser. The server records the
            user agent, source IP, and last-used timestamp for each
            session. You can view and revoke them from the settings page.
          </li>
          <li>
            <strong>Repository contents:</strong> files, commit metadata
            (author name + email + timestamp + message), branches, tags,
            file locks. Whatever you push.
          </li>
          <li>
            <strong>Audit log entries:</strong> a server-side trace at INFO
            and WARN level captures auth events (login attempts, bootstrap,
            token mint, force-unlock). These rotate per the operator's log
            retention policy and never include passwords or token plaintext.
          </li>
        </ul>
      </Section>

      <Section title="What we do not store">
        <ul>
          <li>Browser fingerprints, telemetry beacons, or analytics cookies.</li>
          <li>Page-load tracking or behavioral profiling.</li>
          <li>
            Any third-party cookies. The only cookie set by the web UI is{' '}
            <code>forge_session</code>, a random session token, scoped to
            this origin only.
          </li>
          <li>Plain-text passwords or PATs (only argon2id hashes).</li>
        </ul>
      </Section>

      <Section title="Cookies">
        <p>
          The web UI sets exactly one cookie:{' '}
          <code>
            forge_session=fses_…; Path=/; HttpOnly; Secure; SameSite=Strict
          </code>
          . It carries the random session token that authenticates your
          browser to the gRPC backend. There are no analytics cookies,
          third-party trackers, or fingerprinting scripts.
        </p>
      </Section>

      <Section title="Where the data lives">
        <p>
          All persistent state (the SQLite metadata DB, the chunk store,
          the auto-generated TLS certs) lives under{' '}
          <code>&lt;base_path&gt;</code> on the operator's filesystem. By
          default that's <code>./forge-data/</code> next to the
          forge-server binary, but operators can override it in{' '}
          <code>forge-server.toml</code>. None of that data is replicated
          off-host unless the operator explicitly configures backups.
        </p>
      </Section>

      <Section title="Deletion">
        <p>
          Deleting your user account (server admin only — you can ask via
          the contact link below) cascades to your sessions, PATs, and ACL
          grants. Repository content you authored remains in the commit
          history of any repos that imported it; that's a property of any
          Git-style version control system, not a Forge-specific choice.
        </p>
      </Section>

      <Section title="Questions?">
        <p>
          See the <a href="/contact">Contact page</a> to find out who runs
          this server.
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
