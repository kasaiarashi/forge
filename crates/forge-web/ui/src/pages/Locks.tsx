import { useEffect, useState } from 'react';
import { useParams, Link } from 'react-router-dom';
import {
  Spinner,
  Flash,
  Button,
  Label,
  UnderlineNav,
} from '@primer/react';
import {
  LockIcon,
  UnlockIcon,
  FileIcon,
  PersonIcon,
  ClockIcon,
  CodeIcon,
  GitCommitIcon,
  GearIcon,
  RepoIcon,
} from '@primer/octicons-react';
import type { Lock, Branch } from '../api';
import api from '../api';

function timeAgo(epoch: number): string {
  const date = new Date(epoch * 1000);
  const now = new Date();
  const seconds = Math.floor((now.getTime() - date.getTime()) / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes > 1 ? 's' : ''} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours > 1 ? 's' : ''} ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days} day${days > 1 ? 's' : ''} ago`;
  const months = Math.floor(days / 30);
  return `${months} month${months > 1 ? 's' : ''} ago`;
}

export default function Locks() {
  const { repo = '' } = useParams();
  const [locks, setLocks] = useState<Lock[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [unlocking, setUnlocking] = useState<string | null>(null);
  const [defaultBranch, setDefaultBranch] = useState('main');

  const encRepo = encodeURIComponent(repo);

  const fetchLocks = () => {
    setLoading(true);
    Promise.all([api.getLocks(repo), api.listBranches(repo)])
      .then(([l, branches]) => {
        setLocks(l);
        const main = branches.find((b: Branch) => b.name === 'main') || branches[0];
        if (main) setDefaultBranch(main.name);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    fetchLocks();
  }, [repo]);

  const handleUnlock = async (path: string) => {
    setUnlocking(path);
    try {
      await api.unlockFile(repo, path);
      setLocks((prev) => prev.filter((l) => l.path !== path));
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to unlock');
    } finally {
      setUnlocking(null);
    }
  };

  if (loading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
        <Spinner size="large" />
      </div>
    );
  }

  return (
    <div>
      {/* Repo name header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
        <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}>
          <RepoIcon size={20} />
        </span>
        <Link
          to={`/${encRepo}`}
          style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-accent)', textDecoration: 'none' }}
        >
          {repo}
        </Link>
      </div>

      {/* Repository tabs */}
      <UnderlineNav aria-label="Repository">
        <UnderlineNav.Item
          as={Link}
          to={`/${encRepo}/tree/${encodeURIComponent(defaultBranch)}`}
          icon={CodeIcon}
        >
          Code
        </UnderlineNav.Item>
        <UnderlineNav.Item
          as={Link}
          to={`/${encRepo}/commits/${encodeURIComponent(defaultBranch)}`}
          icon={GitCommitIcon}
        >
          Commits
        </UnderlineNav.Item>
        <UnderlineNav.Item as={Link} to={`/${encRepo}/locks`} aria-current="page" icon={LockIcon}>
          Locks
        </UnderlineNav.Item>
        <UnderlineNav.Item as={Link} to={`/${encRepo}/settings`} icon={GearIcon}>
          Settings
        </UnderlineNav.Item>
      </UnderlineNav>

      {/* Page header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', margin: '16px 0' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><LockIcon size={24} /></span>
          <h2 style={{ fontSize: '20px', fontWeight: 600, margin: 0 }}>File Locks</h2>
          <Label variant="secondary">{locks.length}</Label>
        </div>
        <Button onClick={fetchLocks} size="small">
          Refresh
        </Button>
      </div>

      {error && (
        <div style={{ marginBottom: '16px' }}>
          <Flash variant="danger">{error}</Flash>
        </div>
      )}

      {locks.length === 0 ? (
        <div className="forge-card" style={{ padding: '48px', textAlign: 'center' }}>
          <div style={{ color: 'var(--fg-muted)', marginBottom: '8px', display: 'flex', justifyContent: 'center' }}>
            <UnlockIcon size={40} />
          </div>
          <p style={{ fontSize: '16px', fontWeight: 600, color: 'var(--fg-default)', margin: '0 0 4px 0' }}>
            No active locks
          </p>
          <p style={{ color: 'var(--fg-muted)', fontSize: '14px', margin: 0 }}>
            Files are locked using <code className="text-mono">forge lock</code> to prevent
            conflicts on binary assets.
          </p>
        </div>
      ) : (
        <div className="forge-card">
          {/* Table header */}
          <div style={{
            background: 'var(--bg-subtle)',
            padding: '8px 16px',
            borderBottom: '1px solid var(--border-default)',
            display: 'grid',
            gridTemplateColumns: '1fr 150px 150px 1fr auto',
            gap: '8px',
            alignItems: 'center',
            fontSize: '12px',
            fontWeight: 600,
            color: 'var(--fg-muted)',
          }}>
            <span style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
              <FileIcon size={14} /> Path
            </span>
            <span style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
              <PersonIcon size={14} /> Owner
            </span>
            <span style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
              <ClockIcon size={14} /> Locked
            </span>
            <span>Reason</span>
            <span>Action</span>
          </div>

          {/* Lock rows */}
          {locks.map((lock, i) => (
            <div
              key={lock.path}
              className="file-row"
              style={{
                display: 'grid',
                gridTemplateColumns: '1fr 150px 150px 1fr auto',
                gap: '8px',
                alignItems: 'center',
                padding: '8px 16px',
                borderBottom: i < locks.length - 1 ? '1px solid var(--border-muted)' : 'none',
                fontSize: '14px',
              }}
            >
              <span
                className="text-mono"
                title={lock.path}
                style={{
                  fontSize: '12px',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  display: 'flex',
                  alignItems: 'center',
                  gap: '4px',
                }}
              >
                <span style={{ color: 'var(--fg-warning)', display: 'inline-flex', flexShrink: 0 }}>
                  <LockIcon size={14} />
                </span>
                {lock.path}
              </span>

              <span style={{ fontWeight: 600 }}>{lock.owner}</span>

              <span style={{ color: 'var(--fg-muted)' }}>{timeAgo(lock.created_at)}</span>

              <span style={{
                color: 'var(--fg-muted)',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}>
                {lock.reason || '-'}
              </span>

              <Button
                size="small"
                variant="danger"
                leadingVisual={UnlockIcon}
                onClick={() => handleUnlock(lock.path)}
                disabled={unlocking === lock.path}
              >
                {unlocking === lock.path ? 'Unlocking...' : 'Unlock'}
              </Button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
