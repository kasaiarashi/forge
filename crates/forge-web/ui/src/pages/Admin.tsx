import { useEffect, useState } from 'react';
import type { FC } from 'react';
import {
  Spinner,
  Flash,
  Label,
} from '@primer/react';
import {
  GearIcon,
  ServerIcon,
  ClockIcon,
  GitBranchIcon,
  LockIcon,
  GitCommitIcon,
  DatabaseIcon,
} from '@primer/octicons-react';
import type { ServerInfo } from '../api';
import api from '../api';

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

function StatCard({ icon: Icon, label, value, color = '#1f2328' }: StatCardProps) {
  return (
    <div className="stat-card">
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '4px' }}>
        <span style={{ color: '#656d76', display: 'inline-flex' }}><Icon size={16} /></span>
        <span style={{ fontSize: '12px', color: '#656d76', fontWeight: 600, textTransform: 'uppercase' }}>
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
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    api
      .getServerInfo()
      .then(setInfo)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, []);

  if (loading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
        <Spinner size="large" />
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ padding: '24px 0' }}>
        <Flash variant="danger">
          {error.includes('401') || error.includes('403')
            ? 'Access denied. Admin privileges required.'
            : error}
        </Flash>
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
        borderBottom: '1px solid #d0d7de',
      }}>
        <span style={{ color: '#656d76', display: 'inline-flex' }}><GearIcon size={24} /></span>
        <h2 style={{ fontSize: '20px', fontWeight: 600, margin: 0 }}>
          Server Administration
        </h2>
        <Label variant="accent">Admin</Label>
      </div>

      {/* Server info section */}
      <div style={{ marginBottom: '24px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
          <span style={{ color: '#656d76', display: 'inline-flex' }}><ServerIcon size={20} /></span>
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
          <StatCard icon={GitBranchIcon} label="Branches" value={String(info.branches.length)} color="#0969da" />
          <StatCard
            icon={LockIcon}
            label="Active Locks"
            value={String(info.active_locks)}
            color={info.active_locks > 0 ? '#9a6700' : '#1a7f37'}
          />
          <StatCard icon={GitCommitIcon} label="Total Objects" value={String(info.total_objects)} />
          <StatCard icon={DatabaseIcon} label="Storage" value={formatBytes(info.total_size_bytes)} />
        </div>
      </div>

      {/* Settings section */}
      <div style={{ marginBottom: '24px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
          <span style={{ color: '#656d76', display: 'inline-flex' }}><GearIcon size={20} /></span>
          <h3 style={{ fontSize: '16px', fontWeight: 600, margin: 0 }}>
            Settings
          </h3>
        </div>

        <div className="forge-card">
          <div style={{
            padding: '16px',
            borderBottom: '1px solid #d8dee4',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}>
            <div>
              <div style={{ fontWeight: 600, fontSize: '14px' }}>Server URL</div>
              <div style={{ color: '#656d76', fontSize: '12px' }}>
                The address clients connect to.
              </div>
            </div>
            <code className="text-mono" style={{
              fontSize: '14px',
              color: '#656d76',
              background: '#f6f8fa',
              padding: '4px 8px',
              borderRadius: '6px',
              border: '1px solid #d0d7de',
            }}>
              {window.location.origin}
            </code>
          </div>

          <div style={{
            padding: '16px',
            borderBottom: '1px solid #d8dee4',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}>
            <div>
              <div style={{ fontWeight: 600, fontSize: '14px' }}>Binary storage</div>
              <div style={{ color: '#656d76', fontSize: '12px' }}>
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
              <div style={{ color: '#656d76', fontSize: '12px' }}>
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
