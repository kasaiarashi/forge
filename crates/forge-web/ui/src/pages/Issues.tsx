import { useState, useEffect } from 'react';
import { Link } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import { TextInput, Button, Label, Spinner, Flash } from '@primer/react';
import {
  IssueOpenedIcon,
  SearchIcon,
  CheckIcon,
  CommentIcon,
  IssueClosedIcon,
} from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import api from '../api';
import type { IssueInfo } from '../api';
import { getLabelColor } from '../utils';

function timeAgo(epoch: number): string {
  if (!epoch) return '';
  const seconds = Math.floor((Date.now() - epoch * 1000) / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes !== 1 ? 's' : ''} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours !== 1 ? 's' : ''} ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days} day${days !== 1 ? 's' : ''} ago`;
  const weeks = Math.floor(days / 7);
  if (weeks < 5) return `${weeks} week${weeks !== 1 ? 's' : ''} ago`;
  const months = Math.floor(days / 30);
  return `${months} month${months !== 1 ? 's' : ''} ago`;
}

export default function Issues() {
  const repo = useRepoParam();
  const [filter, setFilter] = useState('');
  const [statusFilter, setStatusFilter] = useState('open');
  const [issues, setIssues] = useState<IssueInfo[]>([]);
  const [openCount, setOpenCount] = useState(0);
  const [closedCount, setClosedCount] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!repo) return;
    setLoading(true);
    api.listIssues(repo, statusFilter)
      .then(resp => {
        setIssues(resp.issues);
        setOpenCount(resp.open_count);
        setClosedCount(resp.closed_count);
      })
      .catch(e => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo, statusFilter]);

  const filtered = filter
    ? issues.filter(i => i.title.toLowerCase().includes(filter.toLowerCase()))
    : issues;

  if (loading) {
    return (
      <div>
        <RepoHeader repo={repo} currentTab="issues" />
        <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
          <Spinner size="large" />
        </div>
      </div>
    );
  }

  return (
    <div>
      <RepoHeader repo={repo} currentTab="issues" />
      <div style={{ maxWidth: '1280px', margin: '0 auto', padding: '0 16px' }}>
        {error && <Flash variant="danger" style={{ marginBottom: 16 }}>{error}</Flash>}

        {/* Header Bar */}
        <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '16px' }}>
          <div style={{ flex: 1, maxWidth: '600px', display: 'flex', gap: '8px' }}>
            <TextInput
              leadingVisual={SearchIcon}
              placeholder="Filter issues..."
              value={filter}
              onChange={e => setFilter(e.target.value)}
              block
              style={{ backgroundColor: 'var(--bg-subtle)' }}
            />
          </div>
          <div style={{ display: 'flex', gap: '8px' }}>
            <Button size="medium" variant="primary" as={Link} to={`/${encodeURIComponent(repo)}/issues/new`}>New issue</Button>
          </div>
        </div>

        {/* Issue List */}
        <div className="forge-card" style={{ border: '1px solid var(--border-default)', borderRadius: '6px' }}>
          <div className="forge-card-header" style={{ display: 'flex', backgroundColor: 'var(--bg-subtle)', padding: '16px', borderBottom: '1px solid var(--border-default)', justifyContent: 'space-between' }}>
            <div style={{ display: 'flex', gap: '16px', fontWeight: 600 }}>
              <span
                style={{ display: 'flex', alignItems: 'center', gap: '8px', color: statusFilter === 'open' ? 'var(--fg-default)' : 'var(--fg-muted)', cursor: 'pointer' }}
                onClick={() => setStatusFilter('open')}
              >
                <IssueOpenedIcon /> {openCount} Open
              </span>
              <span
                style={{ display: 'flex', alignItems: 'center', gap: '8px', color: statusFilter === 'closed' ? 'var(--fg-default)' : 'var(--fg-muted)', cursor: 'pointer' }}
                onClick={() => setStatusFilter('closed')}
              >
                <CheckIcon /> {closedCount} Closed
              </span>
            </div>
            <div style={{ display: 'flex', gap: '16px', color: 'var(--fg-muted)', fontSize: '14px' }}>
              <span>Author</span>
              <span>Label</span>
              <span>Sort</span>
            </div>
          </div>
          <div style={{ display: 'flex', flexDirection: 'column' }}>
            {filtered.length === 0 ? (
              <div style={{ padding: 48, textAlign: 'center', color: 'var(--fg-muted)' }}>
                {issues.length === 0
                  ? 'No issues yet. Create one to get started.'
                  : 'No issues match the current filter.'}
              </div>
            ) : filtered.map((issue, idx) => (
              <div key={issue.id} className="file-row" style={{ display: 'flex', padding: '12px 16px', borderBottom: idx < filtered.length - 1 ? '1px solid var(--border-muted)' : 'none' }}>
                <div style={{ color: issue.status === 'open' ? 'var(--fg-success)' : 'var(--fg-danger)', marginRight: '8px', paddingTop: '4px' }}>
                  {issue.status === 'open' ? <IssueOpenedIcon /> : <IssueClosedIcon />}
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '4px' }}>
                    <Link to={`/${encodeURIComponent(repo)}/issues/${issue.id}`} style={{ fontWeight: 600, fontSize: '16px', color: 'var(--fg-default)', textDecoration: 'none' }} onMouseOver={e => e.currentTarget.style.color = 'var(--fg-accent)'} onMouseOut={e => e.currentTarget.style.color = 'var(--fg-default)'}>
                      {issue.title}
                    </Link>
                    {issue.labels.map(l => (
                      <Label key={l} size="small" style={{ backgroundColor: getLabelColor(l), color: '#fff', border: 'none' }}>{l}</Label>
                    ))}
                  </div>
                  <div style={{ fontSize: '12px', color: 'var(--fg-muted)' }}>
                    #{issue.id} opened {timeAgo(issue.created_at)} by {issue.author}
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'flex-start', color: 'var(--fg-muted)', fontSize: '12px', gap: '4px', paddingTop: '4px' }}>
                  {issue.comment_count > 0 && (
                    <>
                      <CommentIcon /> {issue.comment_count}
                    </>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
