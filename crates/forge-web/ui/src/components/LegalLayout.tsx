// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

import { type ReactNode } from 'react';

interface LegalLayoutProps {
  title: string;
  /** Optional sub-line under the title (e.g. last-updated date). */
  subtitle?: string;
  children: ReactNode;
}

/**
 * Shared shell for the static footer pages — Terms, Privacy, Security,
 * Status, Docs, Contact. Centered narrow column with a clear hierarchy
 * (h1 → h2 → prose) so the pages don't get visually lost in the wider
 * Layout container.
 */
export default function LegalLayout({ title, subtitle, children }: LegalLayoutProps) {
  return (
    <div style={{ maxWidth: '760px', margin: '0 auto', padding: '0 8px' }}>
      <div
        style={{
          borderBottom: '1px solid var(--border-default)',
          paddingBottom: '16px',
          marginBottom: '24px',
        }}
      >
        <h1
          style={{
            fontSize: '32px',
            fontWeight: 600,
            color: 'var(--fg-default)',
            margin: '0',
            lineHeight: 1.2,
          }}
        >
          {title}
        </h1>
        {subtitle && (
          <p
            style={{
              fontSize: '14px',
              color: 'var(--fg-muted)',
              margin: '8px 0 0 0',
            }}
          >
            {subtitle}
          </p>
        )}
      </div>
      <div className="legal-prose" style={{ color: 'var(--fg-default)', fontSize: '15px', lineHeight: 1.7 }}>
        {children}
      </div>
    </div>
  );
}
