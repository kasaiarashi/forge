// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

import LegalLayout from "../components/LegalLayout";

/**
 * `/docs` — minimal but actually-useful operator + user reference.
 * Quickstart, CLI command list, configuration pointers, and links to the
 * upstream README. Deliberately self-contained: a fresh user landing here
 * after their first login should be able to push their first commit
 * without leaving the page.
 */
export default function DocsPage() {
  return (
    <LegalLayout
      title="Documentation"
      subtitle="Quickstart and reference for the Forge CLI and server."
    >
      <p>
        Forge VCS is a binary-first version control system for Unreal Engine
        projects: BLAKE3 + FastCDC + zstd, file locking, gRPC transport,
        MIT-licensed and self-hosted. The pages below are enough to get you
        committing on your first day.
      </p>

      <Section title="Install the CLI">
        <p>
          Grab the latest <code>forge</code> binary from your operator (or the
          project releases page) and put it on your <code>PATH</code>. Verify
          with:
        </p>
        <Code>{`forge --version`}</Code>
      </Section>

      <Section title="First-time setup">
        <p>
          Log in to your Forge server. The CLI auto-detects whether the URL you
          give it is the web UI or the gRPC endpoint, prompts you to trust the
          server's TLS certificate on the first connect, then stores a
          long-lived personal access token in your OS keychain.
        </p>
        <Code>{`forge login --server https://forge.example.com:9876
# or paste the web URL — it'll auto-switch:
forge login --server https://forge.example.com:3000`}</Code>
        <p>
          Verify the cert fingerprint shown in the prompt against what your
          operator communicated to you (Slack, wiki, voice — any trusted
          channel) before typing <code>y</code>. Subsequent commands use the
          pinned cert silently.
        </p>
      </Section>

      <Section title="Initialize and push">
        <Code>{`# In the project root
forge init
forge add .
forge commit -m "initial commit"

# Wire up the remote
forge remote add origin https://forge.example.com:9876/<your-username>/<repo>
forge push`}</Code>
      </Section>

      <Section title="Day-to-day commands">
        <Table
          rows={[
            [
              "forge status",
              "Working tree status (modified, staged, untracked)",
            ],
            ["forge add <path>", "Stage files for the next commit"],
            ['forge commit -m "msg"', "Commit staged changes"],
            ["forge log", "View commit history"],
            ["forge diff", "Show changes vs. last commit"],
            ["forge branch [name]", "List or create a branch"],
            ["forge switch <branch>", "Switch to a branch"],
            ["forge merge <branch>", "Merge a branch into current"],
            ["forge push", "Push commits to the remote"],
            ["forge pull", "Pull commits from the remote"],
            ["forge clone <url>", "Clone a remote repo"],
          ]}
        />
      </Section>

      <Section title="File locking (Unreal-friendly)">
        <p>
          Binary assets like <code>.uasset</code> and <code>.umap</code> can't
          be merged. Forge locks let you signal to teammates that you're about
          to edit one.
        </p>
        <Code>{`forge lock Content/Maps/MainLevel.umap -m "rebalancing combat zones"
forge locks                                  # see who has what
forge unlock Content/Maps/MainLevel.umap`}</Code>
      </Section>

      <Section title="Personal access tokens">
        <p>
          The PAT minted by <code>forge login</code> has <code>repo:read</code>{" "}
          + <code>repo:write</code> scopes. For CI jobs, mint a narrower token
          from the web UI's settings page (or delete the auto-PAT and re-login
          with <code>--token</code>).
        </p>
        <Code>{`# Use a PAT directly without storing a session
forge login --server https://forge.example.com:9876 --token fpat_xxxxxxxxxxxx`}</Code>
      </Section>

      <Section title="Server administration">
        <p>The server-side CLI is shipped alongside forge-server itself:</p>
        <Code>{`forge-server user add --admin alice --email alice@example.com
forge-server user list
forge-server user reset-password alice
forge-server repo grant alice/game bob write
forge-server repo list-members alice/game`}</Code>
      </Section>

      <Section title="Configuration files">
        <p>
          The two config files you actually edit are{" "}
          <code>forge-server.toml</code> (the gRPC server) and{" "}
          <code>forge-web.toml</code> (the browser-facing HTTPS frontend). Both
          auto-generate sensible defaults on first start. The most relevant
          knobs:
        </p>
        <ul>
          <li>
            <code>[server.tls].auto_generate = true</code> — mint a local CA +
            leaf cert on first run, reuse on every restart. Default on.
          </li>
          <li>
            <code>[actions].enabled</code> — opt in to the workflow engine.
            Default <strong>off</strong> because steps run as shell commands on
            the host.
          </li>
          <li>
            <code>[web.rate_limit]</code> — req/s and burst applied to{" "}
            <code>/api/auth/*</code>.
          </li>
          <li>
            <code>[storage.base_path]</code> — where everything lives on disk.
          </li>
        </ul>
      </Section>

      <Section title="More">
        <p>
          The full source tree (Rust) lives at the upstream repo; see{" "}
          <code>README.md</code> for design notes and <code>CLAUDE.md</code> for
          the architectural overview.
        </p>
      </Section>
    </LegalLayout>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section style={{ marginTop: "32px" }}>
      <h2
        style={{
          fontSize: "20px",
          fontWeight: 600,
          color: "var(--fg-default)",
          margin: "0 0 12px 0",
          paddingBottom: "8px",
          borderBottom: "1px solid var(--border-muted)",
        }}
      >
        {title}
      </h2>
      {children}
    </section>
  );
}

function Code({ children }: { children: string }) {
  return (
    <pre
      style={{
        background: "var(--bg-canvas-inset)",
        border: "1px solid var(--border-muted)",
        borderRadius: "6px",
        padding: "12px 16px",
        fontSize: "13px",
        lineHeight: 1.5,
        overflowX: "auto",
        margin: "12px 0",
      }}
    >
      <code>{children}</code>
    </pre>
  );
}

function Table({ rows }: { rows: [string, string][] }) {
  return (
    <table
      style={{
        width: "100%",
        borderCollapse: "collapse",
        margin: "12px 0",
        fontSize: "14px",
      }}
    >
      <tbody>
        {rows.map(([cmd, desc], i) => (
          <tr key={i} style={{ borderTop: "1px solid var(--border-muted)" }}>
            <td
              style={{
                padding: "8px 12px",
                fontFamily: "var(--fontStack-monospace, monospace)",
                whiteSpace: "nowrap",
                color: "var(--fg-default)",
              }}
            >
              {cmd}
            </td>
            <td style={{ padding: "8px 12px", color: "var(--fg-muted)" }}>
              {desc}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
