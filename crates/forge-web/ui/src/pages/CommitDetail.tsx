import { useEffect, useState } from 'react';
import { useParams, Link } from 'react-router-dom';
import {
  Spinner,
  Flash,
  Label,
} from '@primer/react';
import {
  GitCommitIcon,
  DiffAddedIcon,
  DiffRemovedIcon,
  DiffModifiedIcon,
  FileIcon,
  CopyIcon,
  RepoIcon,
} from '@primer/octicons-react';
import type { CommitDetail as CommitDetailType, DiffFile } from '../api';
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

function StatusIcon({ status }: { status: DiffFile['change_type'] }) {
  const colorMap: Record<string, string> = {
    added: 'var(--fg-success)',
    deleted: 'var(--fg-danger)',
    modified: 'var(--fg-warning)',
  };
  const color = colorMap[status] || 'var(--fg-warning)';
  const iconMap: Record<string, typeof DiffModifiedIcon> = {
    added: DiffAddedIcon,
    deleted: DiffRemovedIcon,
    modified: DiffModifiedIcon,
  };
  const Icon = iconMap[status] || DiffModifiedIcon;
  return (
    <span style={{ color, display: 'inline-flex', flexShrink: 0 }}>
      <Icon size={16} />
    </span>
  );
}

export default function CommitDetail() {
  const { repo = '', hash = '' } = useParams();
  const [commit, setCommit] = useState<CommitDetailType | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [copied, setCopied] = useState(false);

  const encRepo = encodeURIComponent(repo);

  useEffect(() => {
    setLoading(true);
    setError('');
    api
      .getCommit(repo, hash)
      .then(setCommit)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo, hash]);

  const handleCopy = () => {
    if (commit?.commit) {
      navigator.clipboard.writeText(commit.commit.hash);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
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

  if (!commit || !commit.commit) return null;

  const info = commit.commit;
  const files = commit.changes;
  const parentHash = info.parent_hashes.length > 0 ? info.parent_hashes[0] : null;

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

      {/* Commit header */}
      <div className="forge-card" style={{ marginBottom: '16px' }}>
        {/* Message */}
        <div style={{
          background: 'var(--bg-subtle)',
          padding: '16px',
          borderBottom: '1px solid var(--border-default)',
        }}>
          <h2 style={{
            fontSize: '20px',
            fontWeight: 600,
            margin: '0 0 8px 0',
            wordBreak: 'break-word',
          }}>
            {info.message}
          </h2>

          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', flexWrap: 'wrap' }}>
            <div className="avatar-circle avatar-circle-sm">
              {info.author_name.charAt(0).toUpperCase()}
            </div>
            <span style={{ fontWeight: 600, fontSize: '14px' }}>{info.author_name}</span>
            <span style={{ color: 'var(--fg-muted)', fontSize: '14px' }}>
              committed {timeAgo(info.timestamp)}
            </span>
          </div>
        </div>

        {/* Commit metadata */}
        <div style={{ padding: '8px 16px' }}>
          <div style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            flexWrap: 'wrap',
            gap: '8px',
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><GitCommitIcon size={16} /></span>
              <span style={{ fontSize: '14px', color: 'var(--fg-muted)' }}>Commit</span>
              <span className="text-mono" style={{ fontSize: '14px', fontWeight: 600 }}>
                {info.hash.slice(0, 7)}
              </span>
              <button
                onClick={handleCopy}
                style={{
                  background: 'none',
                  border: 'none',
                  cursor: 'pointer',
                  padding: '2px',
                  display: 'flex',
                  alignItems: 'center',
                  color: copied ? 'var(--fg-success)' : 'var(--fg-muted)',
                }}
              >
                <CopyIcon size={14} />
              </button>
            </div>
            {parentHash && (
              <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                <span style={{ fontSize: '14px', color: 'var(--fg-muted)' }}>Parent:</span>
                <Link
                  to={`/${encRepo}/commit/${parentHash}`}
                  className="text-mono"
                  style={{ fontSize: '14px', color: 'var(--fg-accent)', textDecoration: 'none' }}
                >
                  {parentHash.slice(0, 7)}
                </Link>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Diff stats summary */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px', flexWrap: 'wrap' }}>
        <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><FileIcon size={16} /></span>
        <span style={{ fontWeight: 600, fontSize: '14px' }}>
          {files.length} file{files.length !== 1 ? 's' : ''} changed
        </span>
      </div>

      {/* File changes list */}
      <div className="forge-card">
        {files.map((file, i) => (
          <div
            key={file.path}
            className="file-row"
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: '8px',
              padding: '8px 16px',
              borderBottom: i < files.length - 1 ? '1px solid var(--border-muted)' : 'none',
            }}
          >
            <StatusIcon status={file.change_type} />

            <span
              className="text-mono"
              style={{
                flex: 1,
                fontSize: '14px',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
              }}
            >
              {file.path}
            </span>

            <Label
              size="small"
              variant={
                file.change_type === 'added'
                  ? 'success'
                  : file.change_type === 'deleted'
                    ? 'danger'
                    : 'attention'
              }
            >
              {file.change_type}
            </Label>
          </div>
        ))}

        {files.length === 0 && (
          <div style={{ padding: '24px', textAlign: 'center', color: 'var(--fg-muted)' }}>
            No files changed in this commit.
          </div>
        )}
      </div>
    </div>
  );
}
