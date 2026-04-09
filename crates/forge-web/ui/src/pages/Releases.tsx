import { useState, useEffect } from 'react';
import { useRepoParam } from '../hooks/useRepoParam';
import { Flash, Spinner, Label } from '@primer/react';
import { TagIcon, PackageIcon } from '@primer/octicons-react';
import api from '../api';
import type { ReleaseInfo } from '../api';
import RepoHeader from '../components/RepoHeader';

function formatTime(ts: number): string {
  if (!ts) return '';
  return new Date(ts * 1000).toLocaleDateString();
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export default function Releases() {
  const repo = useRepoParam();
  const [releases, setReleases] = useState<ReleaseInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!repo) return;
    api.listReleases(repo)
      .then(setReleases)
      .catch(e => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo]);

  if (loading) return <div style={{ padding: 32, textAlign: 'center' }}><Spinner /></div>;
  if (error) return <Flash variant="danger">{error}</Flash>;

  return (
    <div>
      <RepoHeader repo={repo || ''} currentTab="releases" />

      <h2 style={{ marginTop: 16 }}>Releases</h2>

      {releases.length === 0 ? (
        <div style={{ padding: 48, textAlign: 'center', color: 'var(--fg-muted)' }}>
          No releases yet. Create a workflow with a release step to publish releases.
        </div>
      ) : (
        <div>
          {releases.map(release => (
            <div key={release.id} style={{
              border: '1px solid var(--border-default)', borderRadius: 6,
              padding: 20, marginBottom: 16,
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 8 }}>
                <TagIcon size={20} />
                <h3 style={{ margin: 0 }}>{release.name}</h3>
                <Label>{release.tag}</Label>
              </div>
              <div style={{ fontSize: 13, color: 'var(--fg-muted)', marginBottom: 12 }}>
                Published {formatTime(release.created_at)}
                {release.run_id > 0 && <span> from run #{release.run_id}</span>}
              </div>
              {release.artifacts.length > 0 && (
                <div>
                  <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 4, display: 'flex', alignItems: 'center', gap: 6 }}>
                    <PackageIcon size={14} /> Assets
                  </div>
                  {release.artifacts.map(a => (
                    <div key={a.id} style={{
                      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                      padding: '6px 12px', fontSize: 13,
                      borderBottom: '1px solid var(--border-muted)',
                    }}>
                      <span>{a.name}</span>
                      <span style={{ color: 'var(--fg-muted)' }}>{formatSize(a.size_bytes)}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
