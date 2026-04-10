import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import {
  Spinner,
  Flash,
  Button,
  Label,
  TextInput,
  FormControl,
  Avatar,
} from '@primer/react';
import {
  LockIcon,
  UnlockIcon,
  FileIcon,
  PlusIcon,
} from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import type { Lock, Branch } from '../api';
import api, { repoPath } from '../api';

function timeAgo(epoch: number): string {
  const date = new Date(epoch * 1000);
  const now = new Date();
  const seconds = Math.floor((now.getTime() - date.getTime()) / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  return `${months}mo ago`;
}

function formatDate(epoch: number): string {
  return new Date(epoch * 1000).toLocaleDateString(undefined, {
    year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit',
  });
}

/** Get the file name from a path for display. */
function fileName(path: string): string {
  const parts = path.split('/');
  return parts[parts.length - 1] || path;
}

/** Get the directory portion of a path. */
function dirName(path: string): string {
  const idx = path.lastIndexOf('/');
  return idx > 0 ? path.slice(0, idx) + '/' : '';
}

export default function Locks() {
  const repo = useRepoParam();
  const [locks, setLocks] = useState<Lock[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [unlocking, setUnlocking] = useState<string | null>(null);
  const [defaultBranch, setDefaultBranch] = useState('main');
  const [showAcquire, setShowAcquire] = useState(false);
  const [newLockPath, setNewLockPath] = useState('');
  const [newLockReason, setNewLockReason] = useState('');
  const [acquiring, setAcquiring] = useState(false);
  const [acquireError, setAcquireError] = useState('');
  const [filter, setFilter] = useState('');

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

  const handleAcquire = async () => {
    if (!newLockPath.trim()) return;
    setAcquiring(true);
    setAcquireError('');
    try {
      const res = await api.acquireLock(repo, newLockPath.trim(), newLockReason.trim());
      if (!res.granted) {
        setAcquireError('Lock already held by another user');
      } else {
        setNewLockPath('');
        setNewLockReason('');
        setShowAcquire(false);
        fetchLocks();
      }
    } catch (e) {
      setAcquireError(e instanceof Error ? e.message : 'Failed to acquire lock');
    } finally {
      setAcquiring(false);
    }
  };

  if (loading) {
    return (
      <div>
        <RepoHeader repo={repo} currentTab="locks" activeBranch={defaultBranch} />
        <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
          <Spinner size="large" />
        </div>
      </div>
    );
  }

  const encRepo = repoPath(repo);
  const encBranch = encodeURIComponent(defaultBranch);

  // Filter locks
  const filtered = filter
    ? locks.filter((l) => l.path.toLowerCase().includes(filter.toLowerCase()) || l.owner.toLowerCase().includes(filter.toLowerCase()))
    : locks;

  // Group by owner for the summary
  const ownerCounts: Record<string, number> = {};
  for (const l of locks) {
    ownerCounts[l.owner] = (ownerCounts[l.owner] || 0) + 1;
  }
  const owners = Object.entries(ownerCounts).sort((a, b) => b[1] - a[1]);

  return (
    <div>
      <RepoHeader repo={repo} currentTab="locks" activeBranch={defaultBranch} />

      {/* Page header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: '16px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><LockIcon size={24} /></span>
          <h2 style={{ fontSize: '20px', fontWeight: 600, margin: 0 }}>File Locks</h2>
          <Label variant="secondary">{locks.length}</Label>
        </div>
        <div style={{ display: 'flex', gap: '8px' }}>
          <Button size="small" onClick={fetchLocks}>Refresh</Button>
          <Button size="small" variant="primary" onClick={() => setShowAcquire(!showAcquire)}>
            <PlusIcon size={16} /> Lock file
          </Button>
        </div>
      </div>

      {/* Acquire lock form (toggled) */}
      {showAcquire && (
        <div className="forge-card" style={{ marginBottom: '16px' }}>
          <div style={{ padding: '16px' }}>
            <div style={{ display: 'flex', gap: '8px', alignItems: 'flex-end' }}>
              <FormControl style={{ flex: 1 }}>
                <FormControl.Label>File path</FormControl.Label>
                <TextInput
                  value={newLockPath}
                  onChange={(e) => setNewLockPath(e.target.value)}
                  placeholder="Content/Maps/MainLevel.umap"
                  size="small"
                  block
                />
              </FormControl>
              <FormControl style={{ flex: 1 }}>
                <FormControl.Label>Reason (optional)</FormControl.Label>
                <TextInput
                  value={newLockReason}
                  onChange={(e) => setNewLockReason(e.target.value)}
                  placeholder="Editing level layout"
                  size="small"
                  block
                />
              </FormControl>
              <Button size="small" variant="primary" onClick={handleAcquire} disabled={acquiring || !newLockPath.trim()}>
                {acquiring ? 'Locking...' : 'Acquire'}
              </Button>
              <Button size="small" onClick={() => setShowAcquire(false)}>Cancel</Button>
            </div>
            {acquireError && <Flash variant="danger" style={{ marginTop: '8px' }}>{acquireError}</Flash>}
          </div>
        </div>
      )}

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
            Binary files like <code>.uasset</code> and <code>.umap</code> should be locked before editing to prevent conflicts.
          </p>
        </div>
      ) : (
        <div style={{ display: 'flex', gap: '24px' }}>
          {/* Main lock list */}
          <div style={{ flex: 1, minWidth: 0 }}>
            {/* Search/filter */}
            <div style={{ marginBottom: '12px' }}>
              <TextInput
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder="Filter by file path or owner..."
                size="small"
                block
              />
            </div>

            <div className="forge-card">
              {filtered.length === 0 ? (
                <div style={{ padding: '24px', textAlign: 'center', color: 'var(--fg-muted)' }}>
                  No locks match the filter.
                </div>
              ) : (
                filtered.map((lock, i) => (
                  <div
                    key={lock.path}
                    style={{
                      padding: '10px 16px',
                      borderBottom: i < filtered.length - 1 ? '1px solid var(--border-muted)' : 'none',
                      display: 'flex',
                      alignItems: 'center',
                      gap: '12px',
                    }}
                  >
                    {/* Lock icon */}
                    <span style={{ color: 'var(--fg-warning)', display: 'inline-flex', flexShrink: 0 }}>
                      <LockIcon size={16} />
                    </span>

                    {/* File path — clickable link to the file */}
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                        <FileIcon size={14} className="fg-muted" />
                        <Link
                          to={`/${encRepo}/blob/${encBranch}/${lock.path}`}
                          style={{ color: 'var(--fg-accent)', textDecoration: 'none', fontFamily: 'monospace', fontSize: '13px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
                          title={lock.path}
                          onMouseOver={(e) => (e.currentTarget.style.textDecoration = 'underline')}
                          onMouseOut={(e) => (e.currentTarget.style.textDecoration = 'none')}
                        >
                          <span style={{ color: 'var(--fg-muted)' }}>{dirName(lock.path)}</span>
                          <span style={{ fontWeight: 600 }}>{fileName(lock.path)}</span>
                        </Link>
                      </div>
                      <div style={{ fontSize: '12px', color: 'var(--fg-muted)', marginTop: '2px', display: 'flex', gap: '8px', alignItems: 'center' }}>
                        <span>
                          Locked by <span style={{ fontWeight: 600, color: 'var(--fg-default)' }}>{lock.owner}</span>
                        </span>
                        <span title={formatDate(lock.created_at)}>{timeAgo(lock.created_at)}</span>
                        {lock.reason && (
                          <span style={{ fontStyle: 'italic' }}>
                            — {lock.reason}
                          </span>
                        )}
                      </div>
                    </div>

                    {/* Unlock */}
                    <Button
                      size="small"
                      variant="danger"
                      onClick={() => handleUnlock(lock.path)}
                      disabled={unlocking === lock.path}
                    >
                      {unlocking === lock.path ? 'Unlocking...' : 'Unlock'}
                    </Button>
                  </div>
                ))
              )}
            </div>
          </div>

          {/* Sidebar: lock owners summary */}
          <div style={{ width: '220px', flexShrink: 0 }}>
            <div className="forge-card">
              <div className="forge-card-header"><h3 style={{ margin: 0, fontSize: '12px', textTransform: 'uppercase', color: 'var(--fg-muted)' }}>Locked by</h3></div>
              <div style={{ padding: '8px 0' }}>
                {owners.map(([owner, count]) => (
                  <div
                    key={owner}
                    style={{ padding: '6px 16px', display: 'flex', alignItems: 'center', gap: '8px', cursor: 'pointer' }}
                    onClick={() => setFilter(filter === owner ? '' : owner)}
                  >
                    <Avatar
                      src={`https://github.com/identicons/${owner}.png`}
                      size={20}
                    />
                    <span style={{ flex: 1, fontWeight: 600, fontSize: '13px', color: filter === owner ? 'var(--fg-accent)' : 'var(--fg-default)' }}>
                      {owner}
                    </span>
                    <Label size="small" variant="secondary">{count}</Label>
                  </div>
                ))}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
