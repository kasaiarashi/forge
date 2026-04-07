import { useState, useEffect } from 'react';
import { useParams, Link } from 'react-router-dom';
import { Button, Spinner, Flash, Label } from '@primer/react';
import { IssueOpenedIcon, IssueClosedIcon } from '@primer/octicons-react';
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

export default function IssueDetail() {
  const { repo = '', id = '' } = useParams();
  const issueId = parseInt(id, 10);
  
  const [issue, setIssue] = useState<IssueInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [updating, setUpdating] = useState(false);

  useEffect(() => {
    if (!repo || isNaN(issueId)) return;
    setLoading(true);
    api.getIssue(repo, issueId)
      .then(setIssue)
      .catch(e => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo, issueId]);

  const toggleStatus = async () => {
    if (!issue) return;
    setUpdating(true);
    const newStatus = issue.status === 'open' ? 'closed' : 'open';
    try {
      await api.updateIssue(repo, issueId, { status: newStatus });
      setIssue({ ...issue, status: newStatus });
    } catch (e: any) {
      setError(e.message || 'Failed to update issue');
    } finally {
      setUpdating(false);
    }
  };

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

  if (error || !issue) {
    return (
      <div>
        <RepoHeader repo={repo} currentTab="issues" />
        <div style={{ maxWidth: '1280px', margin: '0 auto', padding: '0 16px' }}>
          <Flash variant="danger">{error || 'Issue not found'}</Flash>
        </div>
      </div>
    );
  }

  const isOpen = issue.status === 'open';

  return (
    <div>
      <RepoHeader repo={repo} currentTab="issues" />
      <div style={{ maxWidth: '1280px', margin: '0 auto', padding: '0 16px' }}>
        
        {/* Header */}
        <div style={{ marginBottom: '24px' }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: '8px' }}>
            <h1 style={{ fontSize: '32px', fontWeight: 400, color: 'var(--fg-default)', margin: 0, lineHeight: 1.25 }}>
              {issue.title} <span style={{ color: 'var(--fg-muted)', fontWeight: 300 }}>#{issue.id}</span>
            </h1>
            <Button as={Link} to={`/${encodeURIComponent(repo)}/issues/new`} variant="primary" size="small">New issue</Button>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', fontSize: '14px', color: 'var(--fg-muted)' }}>
            <div style={{ 
              display: 'inline-flex', alignItems: 'center', gap: '4px',
              padding: '5px 12px', borderRadius: '2em', fontWeight: 500, color: '#fff',
              backgroundColor: isOpen ? 'var(--fg-success)' : 'var(--fg-danger)' 
            }}>
              {isOpen ? <IssueOpenedIcon /> : <IssueClosedIcon />}
              {isOpen ? 'Open' : 'Closed'}
            </div>
            <span>
              <span style={{ fontWeight: 600, color: 'var(--fg-default)' }}>{issue.author}</span> opened this issue {timeAgo(issue.created_at)}
              {' '}· {issue.comment_count} comment{issue.comment_count !== 1 ? 's' : ''}
            </span>
          </div>
        </div>
        
        <hr style={{ border: 'none', borderBottom: '1px solid var(--border-default)', margin: '0 0 24px 0' }} />

        <div style={{ display: 'flex', gap: '24px' }}>
          {/* Main timeline/body */}
          <div style={{ flex: 1 }}>
            <div className="forge-card" style={{ border: '1px solid var(--border-default)', borderRadius: '6px' }}>
              <div className="forge-card-header" style={{ backgroundColor: 'var(--bg-subtle)', padding: '16px', borderBottom: '1px solid var(--border-default)' }}>
                <span style={{ fontWeight: 600, color: 'var(--fg-default)' }}>{issue.author}</span>
                <span style={{ color: 'var(--fg-muted)', marginLeft: '8px' }}>commented {timeAgo(issue.created_at)}</span>
              </div>
              <div style={{ padding: '16px', color: 'var(--fg-default)', fontSize: '14px', whiteSpace: 'pre-wrap' }}>
                {issue.body || <span style={{ color: 'var(--fg-muted)', fontStyle: 'italic' }}>No description provided.</span>}
              </div>
            </div>

            <hr style={{ border: 'none', borderBottom: '2px solid var(--border-muted)', margin: '32px 0' }} />

            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '8px' }}>
              <Button onClick={toggleStatus} disabled={updating}>
                {updating ? 'Updating...' : (isOpen ? 'Close issue' : 'Reopen issue')}
              </Button>
            </div>
          </div>

          {/* Sidebar */}
          <div style={{ width: '256px', flexShrink: 0 }}>
            <div style={{ borderBottom: '1px solid var(--border-muted)', paddingBottom: '16px', marginBottom: '16px' }}>
              <h3 style={{ fontSize: '12px', fontWeight: 600, color: 'var(--fg-muted)', marginBottom: '8px', textTransform: 'uppercase' }}>Labels</h3>
              {issue.labels && issue.labels.length > 0 ? (
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: '4px' }}>
                  {issue.labels.map(l => (
                    <Label key={l} style={{ backgroundColor: getLabelColor(l), color: '#fff', border: 'none' }}>{l}</Label>
                  ))}
                </div>
              ) : (
                <span style={{ fontSize: '12px', color: 'var(--fg-muted)' }}>None yet</span>
              )}
            </div>
          </div>
        </div>

      </div>
    </div>
  );
}
