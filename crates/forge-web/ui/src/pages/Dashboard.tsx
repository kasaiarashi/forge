import { useEffect, useState } from 'react';
import { Link } from 'react-router-dom';
import {
  TextInput,
  Spinner,
  Flash,
  Label,
  Button,
  Dialog,
  FormControl,
} from '@primer/react';
import {
  RepoIcon,
  SearchIcon,
  PlusIcon,
  GitBranchIcon,
  ClockIcon,
} from '@primer/octicons-react';
import type { RepoInfo, User } from '../api';
import api from '../api';

function timeAgo(epoch: number): string {
  if (!epoch) return '';
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

export default function Dashboard() {
  const [repos, setRepos] = useState<RepoInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [filter, setFilter] = useState('');
  const [user, setUser] = useState<User | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState('');
  const [newDesc, setNewDesc] = useState('');
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    Promise.all([api.listRepos(), api.me()])
      .then(([r, u]) => {
        setRepos(r);
        setUser(u);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, []);

  const handleCreate = async () => {
    if (!newName.trim()) return;
    setCreating(true);
    try {
      await api.createRepo(newName.trim(), newDesc.trim());
      const updated = await api.listRepos();
      setRepos(updated);
      setShowCreate(false);
      setNewName('');
      setNewDesc('');
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create repository');
    } finally {
      setCreating(false);
    }
  };

  const filtered = repos.filter((r) =>
    r.name.toLowerCase().includes(filter.toLowerCase()) ||
    r.description.toLowerCase().includes(filter.toLowerCase())
  );

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
        <Flash variant="danger">{error}</Flash>
      </div>
    );
  }

  return (
    <div>
      {/* Page header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: '16px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <h2 style={{ fontSize: '20px', fontWeight: 600, margin: 0 }}>
            Repositories
          </h2>
          <Label variant="secondary">{repos.length}</Label>
        </div>
        {user?.is_admin && (
          <Button variant="primary" leadingVisual={PlusIcon} onClick={() => setShowCreate(true)}>
            New repository
          </Button>
        )}
      </div>

      {/* Search */}
      <div style={{ marginBottom: '16px' }}>
        <TextInput
          leadingVisual={SearchIcon}
          placeholder="Find a repository..."
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          block
        />
      </div>

      {/* Repository list */}
      {filtered.length === 0 ? (
        <div className="forge-card" style={{ padding: '48px', textAlign: 'center' }}>
          <div style={{ color: '#656d76', marginBottom: '8px', display: 'flex', justifyContent: 'center' }}>
            <RepoIcon size={40} />
          </div>
          <p style={{ fontSize: '16px', fontWeight: 600, color: '#1f2328', margin: '0 0 4px 0' }}>
            {repos.length === 0 ? 'No repositories yet' : 'No matching repositories'}
          </p>
          <p style={{ color: '#656d76', fontSize: '14px', margin: 0 }}>
            {repos.length === 0
              ? 'Create a repository to get started with Forge VCS.'
              : 'Try a different search term.'}
          </p>
        </div>
      ) : (
        <div className="forge-card">
          {filtered.map((repo, i) => (
            <div
              key={repo.name}
              className="file-row"
              style={{
                display: 'flex',
                alignItems: 'flex-start',
                gap: '12px',
                padding: '16px',
                borderBottom: i < filtered.length - 1 ? '1px solid #d8dee4' : 'none',
              }}
            >
              <span style={{ color: '#656d76', display: 'inline-flex', marginTop: '2px', flexShrink: 0 }}>
                <RepoIcon size={16} />
              </span>

              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '4px', flexWrap: 'wrap' }}>
                  <Link
                    to={`/${encodeURIComponent(repo.name)}`}
                    style={{
                      fontWeight: 600,
                      fontSize: '16px',
                      color: '#0969da',
                      textDecoration: 'none',
                    }}
                    onMouseOver={(e) => (e.currentTarget.style.textDecoration = 'underline')}
                    onMouseOut={(e) => (e.currentTarget.style.textDecoration = 'none')}
                  >
                    {repo.name}
                  </Link>
                  {repo.default_branch && (
                    <Label size="small" variant="secondary">
                      <span style={{ display: 'inline-flex', alignItems: 'center', gap: '2px' }}>
                        <GitBranchIcon size={12} />
                        {repo.branch_count}
                      </span>
                    </Label>
                  )}
                </div>

                {repo.description && (
                  <p style={{ color: '#656d76', fontSize: '14px', margin: '0 0 8px 0', lineHeight: 1.5 }}>
                    {repo.description}
                  </p>
                )}

                <div style={{ display: 'flex', alignItems: 'center', gap: '16px', fontSize: '12px', color: '#656d76', flexWrap: 'wrap' }}>
                  {repo.last_commit_message && (
                    <span style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                      <ClockIcon size={12} />
                      <span style={{ maxWidth: 300, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {repo.last_commit_message}
                      </span>
                    </span>
                  )}
                  {repo.last_commit_author && (
                    <span>
                      by {repo.last_commit_author}
                    </span>
                  )}
                  {repo.last_commit_time > 0 && (
                    <span>
                      {timeAgo(repo.last_commit_time)}
                    </span>
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Create repo dialog */}
      {showCreate && (
        <Dialog title="Create a new repository" onClose={() => setShowCreate(false)}>
          <div style={{ padding: '16px', display: 'flex', flexDirection: 'column', gap: '16px' }}>
            <FormControl>
              <FormControl.Label>Repository name</FormControl.Label>
              <TextInput
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="my-project"
                block
                autoFocus
              />
            </FormControl>
            <FormControl>
              <FormControl.Label>Description (optional)</FormControl.Label>
              <TextInput
                value={newDesc}
                onChange={(e) => setNewDesc(e.target.value)}
                placeholder="A short description of this repository"
                block
              />
            </FormControl>
            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '8px' }}>
              <Button onClick={() => setShowCreate(false)}>Cancel</Button>
              <Button
                variant="primary"
                onClick={handleCreate}
                disabled={creating || !newName.trim()}
              >
                {creating ? 'Creating...' : 'Create repository'}
              </Button>
            </div>
          </div>
        </Dialog>
      )}
    </div>
  );
}
