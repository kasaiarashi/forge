// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

import { useEffect, useState } from 'react';
import { Spinner } from '@primer/react';
import LegalLayout from '../components/LegalLayout';
import api, { type ServerInfo } from '../api';
import { useAuth } from '../context/AuthContext';

/**
 * `/status` — public-ish operational dashboard. The unauthenticated probe
 * (`/api/auth/initialized`) just confirms the server is reachable; the
 * full server-info call (`/api/server/info`) requires a session, so when
 * a logged-out visitor lands here we show only the up/down line and a
 * gentle nudge to log in for the rest.
 */
export default function StatusPage() {
  const { user } = useAuth();
  const [reachable, setReachable] = useState<boolean | null>(null);
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [error, setError] = useState<string>('');

  useEffect(() => {
    // Lightweight reachability probe — the initialized endpoint is public,
    // so we can show a status pill even for logged-out viewers.
    api
      .isInitialized()
      .then(() => setReachable(true))
      .catch(() => setReachable(false));

    // Detailed metrics require a session.
    if (user) {
      api
        .getServerInfo()
        .then(setInfo)
        .catch((e) => setError(e?.message || 'failed to load server info'));
    }
  }, [user]);

  return (
    <LegalLayout
      title="Status"
      subtitle="Live operational metrics for this Forge instance."
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: '12px',
          padding: '16px 20px',
          border: '1px solid var(--border-default)',
          borderRadius: '6px',
          background: 'var(--bg-default)',
          marginBottom: '24px',
        }}
      >
        <Pill state={reachable} />
        <div>
          <div style={{ fontWeight: 600, color: 'var(--fg-default)' }}>
            {reachable === null
              ? 'Checking…'
              : reachable
                ? 'All systems operational'
                : 'Server is unreachable'}
          </div>
          <div style={{ fontSize: '13px', color: 'var(--fg-muted)' }}>
            {reachable === null
              ? 'Probing /api/auth/initialized'
              : reachable
                ? 'forge-server and forge-web are responding to requests.'
                : 'The web layer cannot reach forge-server. Try again or contact your operator.'}
          </div>
        </div>
      </div>

      {!user && (
        <p style={{ color: 'var(--fg-muted)', fontStyle: 'italic' }}>
          Sign in to see object counts, branches, and active locks.
        </p>
      )}

      {user && error && (
        <p style={{ color: 'var(--fg-danger)' }}>
          Could not load detailed metrics: {error}
        </p>
      )}

      {user && info && (
        <>
          <h2 style={H2}>Server</h2>
          <Grid>
            <Metric label="Version" value={info.version || 'unknown'} />
            <Metric label="Uptime" value={formatUptime(info.uptime_secs)} />
            <Metric
              label="Active locks"
              value={String(info.active_locks)}
            />
            <Metric
              label="Repositories"
              value={String(info.repo_count)}
            />
          </Grid>

          <h2 style={H2}>Storage</h2>
          <Grid>
            <Metric
              label="Tracked objects"
              value={info.total_objects.toLocaleString()}
            />
            <Metric
              label="On-disk size"
              value={formatBytes(info.total_size_bytes)}
            />
          </Grid>
        </>
      )}

      {user && !info && !error && (
        <div style={{ display: 'flex', justifyContent: 'center', padding: '32px 0' }}>
          <Spinner />
        </div>
      )}
    </LegalLayout>
  );
}

const H2: React.CSSProperties = {
  fontSize: '20px',
  fontWeight: 600,
  color: 'var(--fg-default)',
  margin: '32px 0 12px 0',
  paddingBottom: '8px',
  borderBottom: '1px solid var(--border-muted)',
};

function Grid({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fit, minmax(180px, 1fr))',
        gap: '12px',
        marginBottom: '24px',
      }}
    >
      {children}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div
      style={{
        padding: '16px',
        border: '1px solid var(--border-default)',
        borderRadius: '6px',
        background: 'var(--bg-default)',
      }}
    >
      <div style={{ fontSize: '12px', color: 'var(--fg-muted)', textTransform: 'uppercase', letterSpacing: '0.5px' }}>
        {label}
      </div>
      <div style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-default)', marginTop: '4px' }}>
        {value}
      </div>
    </div>
  );
}

function Pill({ state }: { state: boolean | null }) {
  let color = 'var(--fg-muted)';
  if (state === true) color = '#1a7f37';
  if (state === false) color = '#cf222e';
  return (
    <span
      aria-hidden
      style={{
        display: 'inline-block',
        width: '12px',
        height: '12px',
        borderRadius: '50%',
        background: color,
        flexShrink: 0,
      }}
    />
  );
}

function formatUptime(secs: number): string {
  if (!secs || secs < 0) return '—';
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m`;
  return `${secs}s`;
}

function formatBytes(b: number): string {
  if (b === 0) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const i = Math.min(units.length - 1, Math.floor(Math.log2(Math.abs(b)) / 10));
  return `${(b / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 2)} ${units[i]}`;
}
