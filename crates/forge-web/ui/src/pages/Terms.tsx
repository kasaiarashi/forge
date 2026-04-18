// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

import LegalLayout from "../components/LegalLayout";

/**
 * `/terms` — terms of service for the *software itself*. Self-hosted
 * Forge VCS is licensed under MIT; the operator running this particular
 * instance is responsible for any additional acceptable-use policy. We
 * surface that distinction explicitly so users know what they're agreeing
 * to (the BSL 1.1.) versus what they should ask their operator about
 * (workplace conduct, data retention, etc.).
 */
export default function TermsPage() {
  return (
    <LegalLayout title="Terms of Use" subtitle="Last updated: April 9, 2026">
      <p>
        Forge VCS is open-source software released under the BSL 1.1.. By using
        this Forge instance you're agreeing both to the BSL 1.1. terms of the
        software itself <em>and</em> to whatever acceptable-use policy the
        operator of this particular server has set. The two are independent —
        read on for the distinction.
      </p>

      <Section title="The software (BSL 1.1.)">
        <p>
          Forge VCS is provided "as is", without warranty of any kind, express
          or implied, including but not limited to the warranties of
          merchantability, fitness for a particular purpose, and
          noninfringement. In no event shall the authors or copyright holders be
          liable for any claim, damages, or other liability, whether in an
          action of contract, tort, or otherwise, arising from, out of, or in
          connection with the software or the use or other dealings in the
          software.
        </p>
        <p>
          You can read the full license text in the <code>LICENSE</code> file at
          the root of the source tree, or at{" "}
          <a
            href="https://opensource.org/licenses/MIT"
            target="_blank"
            rel="noopener noreferrer"
          >
            opensource.org/licenses/MIT
          </a>
          .
        </p>
      </Section>

      <Section title="This particular Forge instance">
        <p>
          The team or person who runs <em>this</em> server decides what you can
          and can't store on it — what kinds of repositories, which file types,
          retention policy, who's allowed in. Forge VCS ships no centralized
          terms of service; ask your server operator for theirs.
        </p>
        <p>
          If you don't know who runs this server, check the{" "}
          <a href="/contact">Contact page</a> — it tells you exactly who to ask.
        </p>
      </Section>

      <Section title="Your account">
        <p>
          You're responsible for the security of your account credentials. Use a
          unique password, mint personal access tokens (PATs) with the narrowest
          scope your workflow needs, and revoke them from the settings page when
          you're done.
        </p>
        <p>
          Forge VCS uses argon2id for password and token hashing, constant-time
          comparisons for token verification, and TLS for all network traffic
          when <code>[server.tls]</code> is configured (which is the default for
          new installs).
        </p>
      </Section>

      <Section title="Modifications">
        <p>
          The BSL 1.1. lets the operator modify the software. They might have
          changed default behavior, branding, or feature set on this instance.
          The source code for any modifications they distribute must remain
          available under the same MIT terms — but they're under no obligation
          to publish private patches.
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
