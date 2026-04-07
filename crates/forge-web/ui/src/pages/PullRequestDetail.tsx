import { useState, useEffect } from 'react';
import { useParams, Link } from 'react-router-dom';
import { Button, Spinner, Flash, Label } from '@primer/react';
import { GitPullRequestIcon, GitMergeIcon, GitPullRequestClosedIcon } from '@primer/octicons-react';
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

export default function PullRequestDetail() {
  const { repo = '', id = '' } = useParams();
  const prId = parseInt(id, 10);
  
  const [pr, setPr] = useState<PullRequestInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [updating, setUpdating] = useState(false);

  useEffect(() => {
    if (!repo || isNaN(prId)) return;
    setLoading(true);
    api.getPullRequest(repo, prId)
      .then(setPr)
      .catch(e => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo, prId]);

  const updateStatus = async (newStatus: string) => {
    if (!pr) return;
    setUpdating(true);
    try {
      await api.updatePullRequest(repo, prId, { status: newStatus });
      setPr({ ...pr, status: newStatus });
    } catch (e: any) {
      setError(e.message || 'Failed to update pull request');
    } finally {
      setUpdating(false);
    }
  };

  const handleMerge = async () => {
    if (!pr) return;
    setUpdating(true);
    try {
      const result = await api.mergePullRequest(repo, prId);
      if (result.success) {
        setPr({ ...pr, status: 'merged' });
      } else {
        setError(result.error || 'Merge failed');
      }
    } catch (e: any) {
      setError(e.message || 'Failed to merge pull request');
    } finally {
      setUpdating(false);
    }
  };

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

  if (error || !pr) {
    return (
      <div>
        <RepoHeader repo={repo} currentTab="pulls" />
        <div style={{ maxWidth: '1280px', margin: '0 auto', padding: '0 16px' }}>
          <Flash variant="danger">{error || 'Pull request not found'}</Flash>
        </div>
      </div>
    );
  }

  return (
    <div>
      <RepoHeader repo={repo} currentTab="pulls" />
      <div style={{ maxWidth: '1280px', margin: '0 auto', padding: '0 16px' }}>
        
        {/* Header */}
        <div style={{ marginBottom: '24px' }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: '8px' }}>
            <h1 style={{ fontSize: '32px', fontWeight: 400, color: 'var(--fg-default)', margin: 0, lineHeight: 1.25 }}>
              {pr.title} <span style={{ color: 'var(--fg-muted)', fontWeight: 300 }}>#{pr.id}</span>
            </h1>
            <Button as={Link} to={`/${encodeURIComponent(repo)}/pulls/new`} variant="primary" size="small">New pull request</Button>
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', fontSize: '14px', color: 'var(--fg-muted)' }}>
            <div style={{ 
              display: 'inline-flex', alignItems: 'center', gap: '4px',
              padding: '5px 12px', borderRadius: '2em', fontWeight: 500, color: '#fff',
              backgroundColor: prIconColor(pr.status)
            }}>
              <PrIcon status={pr.status} />
              <span style={{ textTransform: 'capitalize' }}>{pr.status}</span>
            </div>
            <span>
              <span style={{ fontWeight: 600, color: 'var(--fg-default)' }}>{pr.author}</span> wants to merge into
              {' '}<code style={{ backgroundColor: 'var(--bg-subtle)', padding: '2px 6px', borderRadius: '4px' }}>{pr.target_branch}</code>
              {' '}from
              {' '}<code style={{ backgroundColor: 'var(--bg-subtle)', padding: '2px 6px', borderRadius: '4px' }}>{pr.source_branch}</code>
            </span>
          </div>
        </div>
        
        <hr style={{ border: 'none', borderBottom: '1px solid var(--border-default)', margin: '0 0 24px 0' }} />

        <div style={{ display: 'flex', gap: '24px' }}>
          {/* Main timeline/body */}
          <div style={{ flex: 1 }}>
            <div className="forge-card" style={{ border: '1px solid var(--border-default)', borderRadius: '6px' }}>
              <div className="forge-card-header" style={{ backgroundColor: 'var(--bg-subtle)', padding: '16px', borderBottom: '1px solid var(--border-default)' }}>
                <span style={{ fontWeight: 600, color: 'var(--fg-default)' }}>{pr.author}</span>
                <span style={{ color: 'var(--fg-muted)', marginLeft: '8px' }}>commented {timeAgo(pr.created_at)}</span>
              </div>
              <div style={{ padding: '16px', color: 'var(--fg-default)', fontSize: '14px', whiteSpace: 'pre-wrap' }}>
                {pr.body || <span style={{ color: 'var(--fg-muted)', fontStyle: 'italic' }}>No description provided.</span>}
              </div>
            </div>

            <hr style={{ border: 'none', borderBottom: '2px solid var(--border-muted)', margin: '32px 0' }} />

            <div style={{ display: 'flex', justifyContent: 'flex-start', gap: '8px', padding: '16px', backgroundColor: 'var(--bg-subtle)', border: '1px solid var(--border-default)', borderRadius: '6px' }}>
              <span style={{ color: 'var(--fg-muted)', marginTop: '8px' }}><GitMergeIcon size={24} /></span>
              <div>
                <h4 style={{ margin: '0 0 4px 0' }}>Merge pull request</h4>
                <p style={{ margin: '0 0 16px 0', fontSize: '14px', color: 'var(--fg-muted)' }}>You can merge this pull request manually.</p>
                <div style={{ display: 'flex', gap: '8px' }}>
                  <Button variant="primary" onClick={handleMerge} disabled={updating || pr.status !== 'open'}>
                    Merge pull request
                  </Button>
                  {pr.status === 'open' && (
                    <Button onClick={() => updateStatus('closed')} disabled={updating}>
                      Close pull request
                    </Button>
                  )}
                  {pr.status === 'closed' && (
                    <Button onClick={() => updateStatus('open')} disabled={updating}>
                      Reopen pull request
                    </Button>
                  )}
                </div>
              </div>
            </div>
          </div>

          {/* Sidebar */}
          <div style={{ width: '256px', flexShrink: 0 }}>
            <div style={{ borderBottom: '1px solid var(--border-muted)', paddingBottom: '16px', marginBottom: '16px' }}>
              <h3 style={{ fontSize: '12px', fontWeight: 600, color: 'var(--fg-muted)', marginBottom: '8px', textTransform: 'uppercase' }}>Labels</h3>
              {pr.labels && pr.labels.length > 0 ? (
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: '4px' }}>
                  {pr.labels.map(l => (
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
