import { useEffect, useState } from 'react';
import { useParams, Link } from 'react-router-dom';
import {
  Spinner,
  Flash,
  Button,
} from '@primer/react';
import {
  GitCommitIcon,
  CopyIcon,
} from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import type { CommitList } from '../api';
import api, { copyToClipboard } from '../api';

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
  const { repo = '', branch = 'main' } = useParams();
  const [commitList, setCommitList] = useState<CommitList | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [page, setPage] = useState(1);
  const [copiedHash, setCopiedHash] = useState('');

  const encRepo = encodeURIComponent(repo);

  useEffect(() => {
    setLoading(true);
    setError('');
    api
      .listCommits(repo, branch, page)
      .then(setCommitList)
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

      <div style={{ marginTop: '16px' }}>
        <h2 style={{ fontSize: '20px', fontWeight: 600, marginBottom: '16px' }}>
          Commits on {branch}
        </h2>

        {Array.from(groups.entries()).map(([date, commits]) => (
          <div key={date} style={{ marginBottom: '16px' }}>
            {/* Date heading */}
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '8px' }}>
              <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><GitCommitIcon size={16} /></span>
              <span style={{ fontWeight: 600, fontSize: '14px', color: 'var(--fg-default)' }}>
                Commits on {date}
              </span>
            </div>

            {/* Commit list for this date */}
            <div className="forge-card">
              {commits.map((commit, i) => (
                <div
                  key={commit.hash}
                  className="file-row"
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    padding: '8px 16px',
                    borderBottom: i < commits.length - 1 ? '1px solid var(--border-muted)' : 'none',
                    gap: '16px',
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
          <div style={{ display: 'flex', justifyContent: 'center', gap: '8px', marginTop: '24px', alignItems: 'center' }}>
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
