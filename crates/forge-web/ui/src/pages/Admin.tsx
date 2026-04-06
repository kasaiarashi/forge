import { useEffect, useState } from 'react';
import type { FC } from 'react';
import { Link } from 'react-router-dom';
import {
  Spinner,
  Flash,
  Label,
  Button,
} from '@primer/react';
import {
  GearIcon,
  ServerIcon,
  ClockIcon,
  GitBranchIcon,
  LockIcon,
  GitCommitIcon,
  DatabaseIcon,
  SignInIcon,
  ShieldLockIcon,
} from '@primer/octicons-react';
import type { ServerInfo } from '../api';
import api from '../api';
import { useAuth } from '../context/AuthContext';

function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  const parts: string[] = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  parts.push(`${mins}m`);
  return parts.join(' ');
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

interface StatCardProps {
  icon: FC<{ size?: number; className?: string }>;
  label: string;
  value: string;
  color?: string;
}

function StatCard({ icon: Icon, label, value, color = 'var(--fg-default)' }: StatCardProps) {
  return (
    <div className="stat-card">
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '4px' }}>
        <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><Icon size={16} /></span>
        <span style={{ fontSize: '12px', color: 'var(--fg-muted)', fontWeight: 600, textTransform: 'uppercase' }}>
          {label}
        </span>
      </div>
      <span style={{ fontSize: '28px', fontWeight: 'bold', color }}>
        {value}
      </span>
    </div>
  );
}

export default function Admin() {
  const { user, loading: authLoading } = useAuth();
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    if (authLoading) return;
    if (!user || !user.is_admin) {
      setLoading(false);
      return;
    }
    api
      .getServerInfo()
      .then(setInfo)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, [user, authLoading]);

  if (authLoading || loading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
        <Spinner size="large" />
      </div>
    );
  }

  if (!user) {
    return (
      <div style={{
        display: 'flex',
        justifyContent: 'center',
        padding: '48px 0',
      }}>
        <div className="forge-card" style={{
          padding: '48px',
          textAlign: 'center',
          maxWidth: 400,
        }}>
          <div style={{ color: 'var(--fg-muted)', marginBottom: '16px', display: 'flex', justifyContent: 'center' }}>
            <SignInIcon size={40} />
          </div>
          <h2 style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-default)', margin: '0 0 8px 0' }}>
            Sign in required
          </h2>
          <p style={{ color: 'var(--fg-muted)', fontSize: '14px', margin: '0 0 16px 0' }}>
            You need to sign in to access the admin panel.
          </p>
          <Button as={Link} to="/login" variant="primary">
            Sign in
          </Button>
        </div>
      </div>
    );
  }

  if (!user.is_admin) {
    return (
      <div style={{
        display: 'flex',
        justifyContent: 'center',
        padding: '48px 0',
      }}>
        <div className="forge-card" style={{
          padding: '48px',
          textAlign: 'center',
          maxWidth: 400,
        }}>
          <div style={{ color: 'var(--fg-danger)', marginBottom: '16px', display: 'flex', justifyContent: 'center' }}>
            <ShieldLockIcon size={40} />
          </div>
          <h2 style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-default)', margin: '0 0 8px 0' }}>
            Access denied
          </h2>
          <p style={{ color: 'var(--fg-muted)', fontSize: '14px', margin: '0 0 16px 0' }}>
            You do not have admin privileges. Contact your server administrator.
          </p>
          <Button as={Link} to="/" variant="default">
            Back to repositories
          </Button>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ padding: '24px 0' }}>
        <Flash variant="danger">{error}</Flash>
      </div>
    );
  }

  if (!info) return null;

  return (
    <div>
      {/* Page header */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: '8px',
        marginBottom: '24px',
        paddingBottom: '16px',
        borderBottom: '1px solid var(--border-default)',
      }}>
        <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><GearIcon size={24} /></span>
        <h2 style={{ fontSize: '20px', fontWeight: 600, margin: 0 }}>
          Server Administration
        </h2>
        <Label variant="accent">Admin</Label>
      </div>

      {/* Server info section */}
      <div style={{ marginBottom: '24px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
          <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><ServerIcon size={20} /></span>
          <h3 style={{ fontSize: '16px', fontWeight: 600, margin: 0 }}>
            Server Information
          </h3>
        </div>

        <div style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))',
          gap: '16px',
        }}>
          <StatCard icon={ServerIcon} label="Version" value={info.version} />
          <StatCard icon={ClockIcon} label="Uptime" value={formatUptime(info.uptime_secs)} />
          <StatCard icon={GitBranchIcon} label="Branches" value={String(info.branches.length)} color="var(--fg-accent)" />
          <StatCard
            icon={LockIcon}
            label="Active Locks"
            value={String(info.active_locks)}
            color={info.active_locks > 0 ? 'var(--fg-warning)' : 'var(--fg-success)'}
          />
          <StatCard icon={GitCommitIcon} label="Total Objects" value={String(info.total_objects)} />
          <StatCard icon={DatabaseIcon} label="Storage" value={formatBytes(info.total_size_bytes)} />
        </div>
      </div>

      {/* Settings section */}
      <div style={{ marginBottom: '24px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
          <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><GearIcon size={20} /></span>
          <h3 style={{ fontSize: '16px', fontWeight: 600, margin: 0 }}>
            Settings
          </h3>
        </div>

        <div className="forge-card">
          <div style={{
            padding: '16px',
            borderBottom: '1px solid var(--border-muted)',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}>
            <div>
              <div style={{ fontWeight: 600, fontSize: '14px' }}>Server URL</div>
              <div style={{ color: 'var(--fg-muted)', fontSize: '12px' }}>
                The address clients connect to.
              </div>
            </div>
            <code className="text-mono" style={{
              fontSize: '14px',
              color: 'var(--fg-muted)',
              background: 'var(--bg-subtle)',
              padding: '4px 8px',
              borderRadius: '6px',
              border: '1px solid var(--border-default)',
            }}>
              {window.location.origin}
            </code>
          </div>

          <div style={{
            padding: '16px',
            borderBottom: '1px solid var(--border-muted)',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}>
            <div>
              <div style={{ fontWeight: 600, fontSize: '14px' }}>Binary storage</div>
              <div style={{ color: 'var(--fg-muted)', fontSize: '12px' }}>
                Total size of all stored objects.
              </div>
            </div>
            <Label variant="secondary">{formatBytes(info.total_size_bytes)}</Label>
          </div>

          <div style={{
            padding: '16px',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}>
            <div>
              <div style={{ fontWeight: 600, fontSize: '14px' }}>File locking</div>
              <div style={{ color: 'var(--fg-muted)', fontSize: '12px' }}>
                Prevents concurrent edits to binary assets.
              </div>
            </div>
            <Label variant="success">Enabled</Label>
          </div>
        </div>
      </div>
    </div>
  );
}
