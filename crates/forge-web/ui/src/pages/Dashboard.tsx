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
  IconButton,
  Avatar,
} from '@primer/react';
import {
  RepoIcon,
  SearchIcon,
  PlusIcon,
  CopyIcon,
  CheckIcon,
  XIcon,
  TelescopeIcon,
  LightBulbIcon,
} from '@primer/octicons-react';
import type { RepoInfo } from '../api';
import api, { repoPath,  copyToClipboard } from '../api';
import { useAuth } from '../context/AuthContext';

function CopyableCodeBlock({ lines, label }: { lines: string[]; label: string }) {
  const [copied, setCopied] = useState(false);
  const text = lines.join('\n');

  const handleCopy = () => {
    copyToClipboard(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div style={{ marginBottom: '16px' }}>
      <div style={{ fontSize: '14px', fontWeight: 600, color: 'var(--fg-default)', marginBottom: '8px' }}>
        {label}
      </div>
      <div style={{
        position: 'relative',
        background: '#161b22',
        borderRadius: '6px',
        padding: '16px',
        fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace',
        fontSize: '13px',
        lineHeight: '20px',
        color: '#e6edf3',
        overflow: 'auto',
      }}>
        <div style={{ position: 'absolute', top: '8px', right: '8px' }}>
          <IconButton
            aria-label="Copy"
            icon={copied ? CheckIcon : CopyIcon}
            size="small"
            variant="invisible"
            onClick={handleCopy}
            style={{ color: '#8b949e' }}
          />
        </div>
        {lines.map((line, i) => (
          <div key={i} style={{ whiteSpace: 'pre' }}>{line}</div>
        ))}
      </div>
    </div>
  );
}

export default function Dashboard() {
  const [repos, setRepos] = useState<RepoInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [filter, setFilter] = useState('');
  const { user } = useAuth();
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState('');
  const [newDesc, setNewDesc] = useState('');
  const [creating, setCreating] = useState(false);
  const [createdRepo, setCreatedRepo] = useState<string | null>(null);

  useEffect(() => {
    api.listRepos()
      .then((r) => setRepos(r))
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, []);

  // Bare gRPC server URL (NOT the web UI origin) — same protocol + hostname,
  // forge-server's default port. Used to build the GitHub-style clone URL
  // displayed in the quick-setup card.
  const serverUrl = `${window.location.protocol}//${window.location.hostname}:9876`;
  // Full `<owner>/<repo>` path for the most-recently-created repo. Empty if
  // nothing was just created.
  const createdRepoPath = createdRepo && user?.username
    ? `${user.username}/${createdRepo}`
    : createdRepo ?? '';
  const createdCloneUrl = createdRepoPath ? `${serverUrl}/${createdRepoPath}` : '';

  const handleCreate = async () => {
    if (!newName.trim()) return;
    setCreating(true);
    try {
      const repoName = newName.trim();
      await api.createRepo(repoName, newDesc.trim());
      const updated = await api.listRepos();
      setRepos(updated);
      setShowCreate(false);
      setCreatedRepo(repoName);
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
    <div style={{ display: 'flex', flexDirection: 'column', gap: '16px', maxWidth: '1280px', margin: '0 auto' }}>
      <div style={{ display: 'flex', gap: '32px' }}>
      
      {/* Left Sidebar: Repositories */}
      <div style={{ width: '320px', flexShrink: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: '8px' }}>
          <h2 style={{ fontSize: '14px', fontWeight: 600, margin: 0, color: 'var(--fg-default)' }}>
            Top Repositories
          </h2>
          {user?.is_admin && (
            <Button size="small" variant="primary" leadingVisual={PlusIcon} onClick={() => setShowCreate(true)}>
              New
            </Button>
          )}
        </div>
        
        <div style={{ marginBottom: '16px' }}>
          <TextInput
            leadingVisual={SearchIcon}
            placeholder="Find a repository..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            block
            size="small"
          />
        </div>

        <ul style={{ listStyle: 'none', padding: 0, margin: 0 }}>
          {filtered.length === 0 ? (
            <li style={{ color: 'var(--fg-muted)', fontSize: '13px', padding: '8px 0' }}>
              No repositories found.
            </li>
          ) : (
            filtered.map((repo) => (
              <li key={repo.name} style={{ display: 'flex', alignItems: 'center', gap: '8px', padding: '8px 0', borderBottom: '1px solid var(--border-muted)' }}>
                <Avatar src={`https://github.com/identicons/${user?.username}.png`} size={16} />
                <Link
                  to={`/${repoPath(repo.name)}`}
                  style={{
                    fontWeight: 500,
                    fontSize: '14px',
                    color: 'var(--fg-default)',
                    textDecoration: 'none',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    flex: 1
                  }}
                  onMouseOver={(e) => (e.currentTarget.style.textDecoration = 'underline')}
                  onMouseOut={(e) => (e.currentTarget.style.textDecoration = 'none')}
                >
                  {repo.name}
                </Link>
                <Label size="small" variant="secondary" style={{ flexShrink: 0 }}>
                  Public
                </Label>
              </li>
            ))
          )}
        </ul>
      </div>

      {/* Main Content: Feed */}
      <div style={{ flex: 1, minWidth: 0 }}>
        
        {/* Quick setup after repo creation */}
        {createdRepo && (
          <div className="forge-card" style={{ marginBottom: '24px', position: 'relative' }}>
            <div className="forge-card-header" style={{ justifyContent: 'space-between' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                <RepoIcon size={16} />
                <span style={{ fontWeight: 600 }}>Quick setup for {createdRepoPath}</span>
              </div>
              <IconButton
                aria-label="Dismiss"
                icon={XIcon}
                variant="invisible"
                size="small"
                onClick={() => setCreatedRepo(null)}
              />
            </div>
            <div style={{ padding: '16px' }}>
              <CopyableCodeBlock
                label="Quick setup — clone and start committing"
                lines={[
                  `forge clone ${createdCloneUrl}`,
                  `cd ${createdRepo}`,
                  'echo "# starter" > README.md',
                  'forge add .',
                  'forge commit -m "first commit"',
                  'forge push',
                ]}
              />
              <CopyableCodeBlock
                label="…or push an existing repository from the command line"
                lines={[
                  `forge remote add origin ${createdCloneUrl}`,
                  'forge push',
                ]}
              />
            </div>
          </div>
        )}

        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: '16px' }}>
          <h2 style={{ fontSize: '20px', fontWeight: 600, margin: 0 }}>
            Home
          </h2>
        </div>

        {repos.length === 0 ? (
          <div className="forge-card" style={{ padding: '48px', textAlign: 'center' }}>
            <div style={{ color: 'var(--fg-muted)', marginBottom: '8px', display: 'flex', justifyContent: 'center' }}>
              <TelescopeIcon size={40} />
            </div>
            <p style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-default)', margin: '0 0 8px 0' }}>
              Welcome to Forge VCS
            </p>
            <p style={{ color: 'var(--fg-muted)', fontSize: '14px', margin: '0 0 16px 0', maxWidth: '400px', marginLeft: 'auto', marginRight: 'auto' }}>
              It looks like you don't have any repositories yet. Create your first repository or connect to an existing one to get started.
            </p>
            {user?.is_admin && (
              <Button variant="primary" leadingVisual={PlusIcon} onClick={() => setShowCreate(true)}>
                Create your first repository
              </Button>
            )}
          </div>
        ) : (
          <div className="forge-card" style={{ padding: '32px', textAlign: 'center' }}>
            <div style={{ color: 'var(--fg-muted)', marginBottom: '16px', display: 'flex', justifyContent: 'center' }}>
              <LightBulbIcon size={32} />
            </div>
            <p style={{ fontSize: '16px', fontWeight: 600, color: 'var(--fg-default)', margin: '0 0 8px 0' }}>
              No recent activity.
            </p>
            <p style={{ color: 'var(--fg-muted)', fontSize: '14px', margin: '0 0 16px 0' }}>
              Push a commit, open a pull request, or acquire a lock — recent activity will show up here.
            </p>
          </div>
        )}
      </div>

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
    </div>
  );
}
