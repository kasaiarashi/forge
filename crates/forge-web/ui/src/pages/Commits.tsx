import { useEffect, useState } from 'react';
import { useParams, Link, useNavigate } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import {
  Spinner,
  Flash,
  Button,
  ActionMenu,
  ActionList,
} from '@primer/react';
import {
  GitCommitIcon,
  GitBranchIcon,
  CopyIcon,
} from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import type { CommitList, Branch } from '../api';
import api, { repoPath,  copyToClipboard } from '../api';

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

function formatDate(epoch: number): string {
  const date = new Date(epoch * 1000);
  return date.toLocaleDateString('en-US', {
    month: 'long',
    day: 'numeric',
    year: 'numeric',
  });
}

import type { CommitSummary } from '../api';

function groupByDate(commits: CommitSummary[]): Map<string, CommitSummary[]> {
  const groups = new Map<string, CommitSummary[]>();
  for (const commit of commits) {
    const dateKey = formatDate(commit.timestamp);
    const existing = groups.get(dateKey) || [];
    existing.push(commit);
    groups.set(dateKey, existing);
  }
  return groups;
}

export default function Commits() {
  const repo = useRepoParam();
  const { branch = 'main' } = useParams<{ branch?: string }>();
  const navigate = useNavigate();
  const [commitList, setCommitList] = useState<CommitList | null>(null);
  const [branches, setBranches] = useState<Branch[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [page, setPage] = useState(1);
  const [copiedHash, setCopiedHash] = useState('');

  const encRepo = repoPath(repo);

  useEffect(() => {
    setLoading(true);
    setError('');
    Promise.all([
      api.listCommits(repo, branch, page),
      api.listBranches(repo),
    ])
      .then(([commits, br]) => {
        setCommitList(commits);
        setBranches(br);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo, branch, page]);

  const handleCopyHash = (hash: string) => {
    copyToClipboard(hash);
    setCopiedHash(hash);
    setTimeout(() => setCopiedHash(''), 2000);
  };

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

  if (!commitList) return null;

  const groups = groupByDate(commitList.commits);
  const perPage = 30;
  const totalPages = Math.ceil(commitList.total / perPage);

  return (
    <div>
      <RepoHeader repo={repo} currentTab="commits" activeBranch={branch} />

      <div style={{ maxWidth: '1280px', margin: '0 auto', padding: '0 var(--space-6)', marginTop: 'var(--space-4)' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)', marginBottom: 'var(--space-4)' }}>
          <h2 style={{ fontSize: '20px', fontWeight: 600, margin: 0 }}>
            Commits
          </h2>
          <ActionMenu>
            <ActionMenu.Button variant="default" size="small">
              <GitBranchIcon size={16} /> {branch}
            </ActionMenu.Button>
            <ActionMenu.Overlay>
              <ActionList>
                {branches.map(b => (
                  <ActionList.Item key={b.name} onSelect={() => navigate(`/${repoPath(repo)}/commits/${encodeURIComponent(b.name)}`)}>
                    {b.name}
                  </ActionList.Item>
                ))}
              </ActionList>
            </ActionMenu.Overlay>
          </ActionMenu>
        </div>

        {Array.from(groups.entries()).map(([date, commits]) => (
          <div key={date} style={{ marginBottom: 'var(--space-4)' }}>
            {/* Date heading */}
            <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)', marginBottom: 'var(--space-2)' }}>
              <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><GitCommitIcon size={16} /></span>
              <span style={{ fontWeight: 600, fontSize: '14px', color: 'var(--fg-default)' }}>
                Commits on {date}
              </span>
            </div>

            {/* Commit list for this date */}
            <div className="forge-card">
              {commits.map((commit) => (
                <div
                  key={commit.hash}
                  className="file-row"
                  style={{
                    display: 'flex',
                    justifyContent: 'space-between',
                    gap: 'var(--space-4)',
                  }}
                >
                  {/* Left side: avatar + message */}
                  <div style={{ display: 'flex', alignItems: 'center', gap: '8px', flex: 1, minWidth: 0 }}>
                    <div className="avatar-circle">
                      {commit.author_name.charAt(0).toUpperCase()}
                    </div>

                    <div style={{ flex: 1, minWidth: 0 }}>
                      <Link
                        to={`/${encRepo}/commit/${commit.hash}`}
                        style={{
                          fontWeight: 600,
                          fontSize: '14px',
                          color: 'var(--fg-default)',
                          textDecoration: 'none',
                          display: 'block',
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                        }}
                        onMouseOver={(e) => (e.currentTarget.style.color = 'var(--fg-accent)')}
                        onMouseOut={(e) => (e.currentTarget.style.color = 'var(--fg-default)')}
                      >
                        {commit.message}
                      </Link>
                      <span style={{ fontSize: '12px', color: 'var(--fg-muted)' }}>
                        {commit.author_name} committed {timeAgo(commit.timestamp)}
                      </span>
                    </div>
                  </div>

                  {/* Right side: hash */}
                  <div style={{ display: 'flex', alignItems: 'center', gap: '4px', flexShrink: 0 }}>
                    <button
                      onClick={() => handleCopyHash(commit.hash)}
                      style={{
                        background: 'none',
                        border: 'none',
                        cursor: 'pointer',
                        padding: '4px',
                        display: 'flex',
                        alignItems: 'center',
                        color: copiedHash === commit.hash ? 'var(--fg-success)' : 'var(--fg-muted)',
                      }}
                      aria-label="Copy commit hash"
                    >
                      <CopyIcon size={14} />
                    </button>
                    <Link
                      to={`/${encRepo}/commit/${commit.hash}`}
                      className="text-mono"
                      style={{
                        fontSize: '12px',
                        color: 'var(--fg-accent)',
                        textDecoration: 'none',
                      }}
                    >
                      {commit.hash.slice(0, 7)}
                    </Link>
                  </div>
                </div>
              ))}
            </div>
          </div>
        ))}

        {/* Pagination */}
        {totalPages > 1 && (
          <div style={{ display: 'flex', justifyContent: 'center', gap: 'var(--space-2)', marginTop: 'var(--space-6)', alignItems: 'center' }}>
            <Button disabled={page <= 1} onClick={() => setPage((p) => p - 1)}>
              Newer
            </Button>
            <span style={{ color: 'var(--fg-muted)', fontSize: '14px' }}>
              Page {page} of {totalPages}
            </span>
            <Button disabled={page >= totalPages} onClick={() => setPage((p) => p + 1)}>
              Older
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
