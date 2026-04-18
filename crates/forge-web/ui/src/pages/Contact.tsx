// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

import { useEffect, useState } from "react";
import LegalLayout from "../components/LegalLayout";
import api, { type UserSummary } from "../api";

/**
 * `/contact` — for self-hosted Forge there is no central support address.
 * The contact page tells the user "ask your operator" and surfaces every
 * server admin we know about so they can actually find one.
 *
 * Falls back to a generic message when the visitor isn't logged in (the
 * admin list endpoint requires a session).
 */
export default function ContactPage() {
  const [admins, setAdmins] = useState<UserSummary[] | null>(null);
  const [loadError, setLoadError] = useState<string>("");

  useEffect(() => {
    // listUsers requires server-admin auth, but admins for the *contact*
    // page should be discoverable. We attempt the call anyway and accept
    // a 401/403 silently — the rest of the page still renders.
    api
      .listUsers()
      .then((users) => setAdmins(users.filter((u) => u.is_server_admin)))
      .catch((e: any) => {
        const msg: string = e?.message || "";
        if (
          msg.includes("401") ||
          msg.includes("403") ||
          msg.includes("login")
        ) {
          setAdmins([]);
        } else {
          setLoadError(msg);
        }
      });
  }, []);

  return (
    <LegalLayout
      title="Contact"
      subtitle="Forge VCS is self-hosted — there is no upstream support team."
    >
      <p>
        This Forge instance is run by whoever deployed it. There's no central
        Forge company; the upstream project is open source and ships no SaaS. So
        "who do I talk to?" depends on <em>your</em> server.
      </p>

      <Section title="Operational issues with this server">
        <p>
          If you can't log in, your push is timing out, the server seems slow,
          your account got disabled, or you spotted what might be a security
          issue: contact the person or team who runs this Forge instance. They
          have access to the server-side logs and the metadata DB.
        </p>
        {loadError && (
          <p style={{ color: "var(--fg-danger)" }}>
            Could not list server admins: {loadError}
          </p>
        )}
        {admins && admins.length > 0 && (
          <>
            <p>The administrators on this server are:</p>
            <ul>
              {admins.map((a) => (
                <li key={a.id}>
                  <strong>{a.display_name || a.username}</strong> (
                  <code>{a.username}</code>)
                  {a.email && (
                    <>
                      {" "}
                      — <a href={`mailto:${a.email}`}>{a.email}</a>
                    </>
                  )}
                </li>
              ))}
            </ul>
          </>
        )}
        {admins && admins.length === 0 && (
          <p>
            <em>
              We can't display the admin list to your current session. Log in
              (or ask anyone with a logged-in session) to see who runs this
              server.
            </em>
          </p>
        )}
      </Section>

      <Section title="Bugs in the Forge software itself">
        <p>
          If you've found a bug in the upstream Forge VCS code (CLI, gRPC
          server, web UI, or the actions engine), the project lives at{" "}
          <a
            href="https://github.com/kasaiarashi/forge"
            target="_blank"
            rel="noopener noreferrer"
          >
            github.com/kasaiarashi/forge
          </a>
          . Open an issue with reproduction steps and your{" "}
          <code>forge --version</code> + <code>forge-server --version</code>.
        </p>
      </Section>

      <Section title="Security disclosures">
        <p>
          For vulnerabilities (in the upstream code or in this particular
          deployment) please follow the{" "}
          <a href="/security">responsible disclosure process</a>. Don't open
          public issues for security reports until a fix has been released.
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
