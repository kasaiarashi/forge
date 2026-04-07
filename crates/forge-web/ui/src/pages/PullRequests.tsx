import { useState, useEffect } from 'react';
import { useParams, Link } from 'react-router-dom';
import { TextInput, Button, Label, Spinner, Flash } from '@primer/react';
import {
  GitPullRequestIcon,
  SearchIcon,
  CheckIcon,
  CommentIcon,
  GitMergeIcon,
  GitPullRequestClosedIcon,
  LightBulbIcon,
} from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import api from '../api';
import type { PullRequestInfo } from '../api';
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

function PrIcon({ status }: { status: string }) {
  switch (status) {
    case 'merged': return <GitMergeIcon />;
    case 'closed': return <GitPullRequestClosedIcon />;
    default: return <GitPullRequestIcon />;
  }
}

function prIconColor(status: string): string {
  switch (status) {
    case 'merged': return 'var(--fg-accent)';
    case 'closed': return 'var(--fg-danger)';
    default: return 'var(--fg-success)';
  }
}

export default function PullRequests() {
  const { repo = '' } = useParams();
  const [filter, setFilter] = useState('');
  const [statusFilter, setStatusFilter] = useState('open');
  const [prs, setPrs] = useState<PullRequestInfo[]>([]);
  const [openCount, setOpenCount] = useState(0);
  const [closedCount, setClosedCount] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!repo) return;
    setLoading(true);
    api.listPullRequests(repo, statusFilter)
      .then(resp => {
        setPrs(resp.pull_requests);
        setOpenCount(resp.open_count);
        setClosedCount(resp.closed_count);
      })
      .catch(e => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo, statusFilter]);

  const filtered = filter
    ? prs.filter(p => p.title.toLowerCase().includes(filter.toLowerCase()))
    : prs;

  if (loading) {
    return (
      <div>
        <RepoHeader repo={repo} currentTab="pulls" />
        <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
          <Spinner size="large" />
        </div>
      </div>
    );
  }

  return (
    <div>
      <RepoHeader repo={repo} currentTab="pulls" />
      <div style={{ maxWidth: '1280px', margin: '0 auto', padding: '0 16px' }}>
        {error && <Flash variant="danger" style={{ marginBottom: 16 }}>{error}</Flash>}

        {/* Header Bar */}
        <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '16px' }}>
          <div style={{ flex: 1, maxWidth: '600px', display: 'flex', gap: '8px' }}>
            <TextInput
              leadingVisual={SearchIcon}
              placeholder="Filter pull requests..."
              value={filter}
              onChange={e => setFilter(e.target.value)}
              block
              style={{ backgroundColor: 'var(--bg-subtle)' }}
            />
          </div>
          <div style={{ display: 'flex', gap: '8px' }}>
            <Button size="medium" variant="primary" leadingVisual={GitPullRequestIcon} as={Link} to={`/${encodeURIComponent(repo)}/pulls/new`}>New pull request</Button>
          </div>
        </div>

        {/* PR List */}
        <div className="forge-card" style={{ border: '1px solid var(--border-default)', borderRadius: '6px' }}>
          <div className="forge-card-header" style={{ display: 'flex', backgroundColor: 'var(--bg-subtle)', padding: '16px', borderBottom: '1px solid var(--border-default)', justifyContent: 'space-between' }}>
            <div style={{ display: 'flex', gap: '16px', fontWeight: 600 }}>
              <span
                style={{ display: 'flex', alignItems: 'center', gap: '8px', color: statusFilter === 'open' ? 'var(--fg-default)' : 'var(--fg-muted)', cursor: 'pointer' }}
                onClick={() => setStatusFilter('open')}
              >
                <GitPullRequestIcon /> {openCount} Open
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
              <div style={{ padding: '80px 48px', textAlign: 'center' }}>
                <div style={{ marginBottom: '16px', color: 'var(--fg-muted)' }}>
                  <GitPullRequestIcon size={32} />
                </div>
                <h3 style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-default)', margin: '0 0 8px 0' }}>
                  There aren't any {statusFilter} pull requests.
                </h3>
                <p style={{ color: 'var(--fg-muted)', margin: 0 }}>
                  You could search all of Forge VCS or try an advanced search.
                </p>
              </div>
            ) : filtered.map((pr, idx) => (
              <div key={pr.id} className="file-row" style={{ display: 'flex', padding: '12px 16px', borderBottom: idx < filtered.length - 1 ? '1px solid var(--border-muted)' : 'none' }}>
                <div style={{ color: prIconColor(pr.status), marginRight: '8px', paddingTop: '4px' }}>
                  <PrIcon status={pr.status} />
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '4px' }}>
                    <Link to={`/${encodeURIComponent(repo)}/pulls/${pr.id}`} style={{ fontWeight: 600, fontSize: '16px', color: 'var(--fg-default)', textDecoration: 'none' }} onMouseOver={e => e.currentTarget.style.color = 'var(--fg-accent)'} onMouseOut={e => e.currentTarget.style.color = 'var(--fg-default)'}>
                      {pr.title}
                    </Link>
                    {pr.labels.map(l => (
                      <Label key={l} size="small" style={{ backgroundColor: getLabelColor(l), color: '#fff', border: 'none' }}>{l}</Label>
                    ))}
                  </div>
                  <div style={{ fontSize: '12px', color: 'var(--fg-muted)' }}>
                    #{pr.id} opened {timeAgo(pr.created_at)} by {pr.author} &middot; {pr.source_branch} &rarr; {pr.target_branch}
                  </div>
                </div>
                <div style={{ display: 'flex', alignItems: 'flex-start', color: 'var(--fg-muted)', fontSize: '12px', gap: '4px', paddingTop: '4px' }}>
                  {pr.comment_count > 0 && (
                    <>
                      <CommentIcon /> {pr.comment_count}
                    </>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>

        <div style={{ textAlign: 'center', marginTop: '32px', fontSize: '14px', color: 'var(--fg-muted)', paddingBottom: '32px' }}>
          <span style={{ marginRight: '8px', position: 'relative', top: '2px' }}><LightBulbIcon size={16} /></span>
          <span style={{ fontWeight: 600, color: 'var(--fg-default)' }}>ProTip!</span> Type <code style={{ padding: '2px 6px', fontFamily: 'ui-monospace, SFMono-Regular, monospace', fontSize: '12px', border: '1px solid var(--border-default)', borderRadius: '6px', backgroundColor: 'var(--bg-subtle)' }}>g</code> <code style={{ padding: '2px 6px', fontFamily: 'ui-monospace, SFMono-Regular, monospace', fontSize: '12px', border: '1px solid var(--border-default)', borderRadius: '6px', backgroundColor: 'var(--bg-subtle)' }}>p</code> on any issue or pull request to go back to the pull request listing page.
        </div>
      </div>
    </div>
  );
}
