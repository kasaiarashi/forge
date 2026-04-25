import { useEffect, useState } from 'react';
import { useRepoParam } from '../hooks/useRepoParam';
import { Link } from 'react-router-dom';
import {
  Spinner,
  Flash,
  Button
} from '@primer/react';
import { GitBranchIcon } from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import api, { repoPath } from '../api';
import type { Branch } from '../api';

export default function Branches() {
  const repo = useRepoParam();
  const [branches, setBranches] = useState<Branch[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [deleting, setDeleting] = useState<string | null>(null);

  const encRepo = repoPath(repo);

  useEffect(() => {
    setLoading(true);
    api.listBranches(repo)
      .then(setBranches)
      .catch(e => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo]);

  const handleDelete = async (branchName: string) => {
    if (!window.confirm(`Are you sure you want to permanently delete branch '${branchName}'?`)) return;
    setDeleting(branchName);
    setError('');
    try {
      await api.deleteBranch(repo, branchName);
      setBranches(branches.filter(b => b.name !== branchName));
    } catch (e: any) {
      setError(`Failed to delete branch: ${e.message}`);
    } finally {
      setDeleting(null);
    }
  };

  if (loading) {
    return (
      <div>
        <RepoHeader repo={repo} currentTab="code" activeBranch="main" />
        <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
          <Spinner size="large" />
        </div>
      </div>
    );
  }

  return (
    <div>
      <RepoHeader repo={repo} currentTab="code" activeBranch="main" />

      <div style={{ maxWidth: '1280px', margin: '0 auto', padding: '0 var(--space-6)', marginTop: 'var(--space-4)' }}>
        {error && (
          <div style={{ marginBottom: '16px' }}>
            <Flash variant="danger">{error}</Flash>
          </div>
        )}

        <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)', marginBottom: 'var(--space-4)' }}>
          <h2 style={{ fontSize: '20px', fontWeight: 600, margin: 0 }}>Branches</h2>
        </div>

        <div className="forge-card">
          <div className="forge-card-header" style={{ fontWeight: 600 }}>All branches</div>
          <div style={{ display: 'flex', flexDirection: 'column' }}>
            {branches.map(b => (
              <div 
                key={b.name} 
                className="file-row" 
                style={{ 
                  display: 'flex', 
                  alignItems: 'center', 
                  justifyContent: 'space-between',
                  padding: '16px' 
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
                  <GitBranchIcon size={16} fill="var(--fg-muted)" />
                  <Link 
                    to={`/${encRepo}/tree/${encodeURIComponent(b.name)}`} 
                    style={{ fontWeight: 600, fontSize: '14px', color: 'var(--fg-accent)', textDecoration: 'none' }}
                  >
                    {b.name}
                  </Link>
                  {b.name === 'main' && (
                    <span style={{ 
                      padding: '2px 8px', 
                      background: 'var(--bg-subtle)', 
                      border: '1px solid var(--border-default)', 
                      borderRadius: '12px', 
                      fontSize: '12px', 
                      color: 'var(--fg-muted)' 
                    }}>
                      Default
                    </span>
                  )}
                </div>

                <div style={{ display: 'flex', gap: '16px', alignItems: 'center' }}>
                  <span style={{ fontSize: '12px', color: 'var(--fg-muted)', fontFamily: 'monospace' }}>
                    {b.head.slice(0, 7)}
                  </span>
                  
                  {b.name !== 'main' ? (
                    <Button 
                      variant="danger" 
                      onClick={() => handleDelete(b.name)} 
                      disabled={deleting === b.name}
                      size="small"
                    >
                      {deleting === b.name ? 'Deleting...' : 'Delete'}
                    </Button>
                  ) : (
                    <div style={{ width: '61px' }}></div> 
                  )}
                </div>
              </div>
            ))}
            
            {branches.length === 0 && (
              <div style={{ padding: '24px', textAlign: 'center', color: 'var(--fg-muted)' }}>
                No branches found.
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
