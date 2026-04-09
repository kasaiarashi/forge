import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import {
  TextInput,
  Button,
  FormControl,
  Flash,
  Spinner,
} from '@primer/react';
import {
  AlertIcon,
  CopyIcon,
} from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import type { RepoInfo, Branch } from '../api';
import api, { copyToClipboard } from '../api';

export default function RepoSettings() {
  const repo = useRepoParam();
  const navigate = useNavigate();

  const [, setRepoInfo] = useState<RepoInfo | null>(null);
  const [defaultBranch, setDefaultBranch] = useState('main');
  const [branches, setBranches] = useState<Branch[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  // General settings state
  const [newName, setNewName] = useState('');
  const [description, setDescription] = useState('');
  const [renameStatus, setRenameStatus] = useState<{ type: 'success' | 'danger'; msg: string } | null>(null);
  const [descStatus, setDescStatus] = useState<{ type: 'success' | 'danger'; msg: string } | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [savingDesc, setSavingDesc] = useState(false);

  // Delete state
  const [deleteConfirm, setDeleteConfirm] = useState('');
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState('');
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);

  // Clone URL copy
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    setLoading(true);
    Promise.all([api.listRepos(), api.listBranches(repo)])
      .then(([repos, br]) => {
        const info = repos.find((r) => r.name === repo);
        if (info) {
          setRepoInfo(info);
          setNewName(info.name);
          setDescription(info.description || '');
        }
        setBranches(br);
        const main = br.find((b: Branch) => b.name === 'main') || br[0];
        if (main) setDefaultBranch(main.name);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo]);

  const handleRename = async () => {
    if (!newName.trim() || newName === repo) return;
    setRenaming(true);
    setRenameStatus(null);
    try {
      await api.updateRepo(repo, { new_name: newName.trim() });
      setRenameStatus({ type: 'success', msg: 'Repository renamed successfully.' });
      setTimeout(() => navigate(`/${encodeURIComponent(newName.trim())}/settings`), 1000);
    } catch (e) {
      setRenameStatus({ type: 'danger', msg: e instanceof Error ? e.message : 'Failed to rename' });
    } finally {
      setRenaming(false);
    }
  };

  const handleSaveDescription = async () => {
    setSavingDesc(true);
    setDescStatus(null);
    try {
      await api.updateRepo(repo, { description });
      setDescStatus({ type: 'success', msg: 'Description updated.' });
    } catch (e) {
      setDescStatus({ type: 'danger', msg: e instanceof Error ? e.message : 'Failed to save' });
    } finally {
      setSavingDesc(false);
    }
  };

  const handleDelete = async () => {
    if (deleteConfirm !== repo) return;
    setDeleting(true);
    setDeleteError('');
    try {
      await api.deleteRepo(repo);
      navigate('/');
    } catch (e) {
      setDeleteError(e instanceof Error ? e.message : 'Failed to delete');
      setDeleting(false);
    }
  };

  // Full GitHub-style clone URL: server + path. The bare server URL is no
  // longer surfaced in the UI — `forge clone` now takes the path inline.
  const serverUrl = `${window.location.protocol}//${window.location.hostname}:9876`;
  const cloneUrl = `${serverUrl}/${repo}`;

  const handleCopyClone = () => {
    copyToClipboard(`forge clone ${cloneUrl}`);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
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

  return (
    <div>
      <RepoHeader repo={repo} currentTab="settings" activeBranch={defaultBranch} />

      <div style={{ marginTop: '24px', maxWidth: '720px' }}>
        <h2 style={{ fontSize: '20px', fontWeight: 600, marginBottom: '24px', color: 'var(--fg-default)' }}>
          General
        </h2>

        {/* Repository name */}
        <div style={{ marginBottom: '24px' }}>
          <FormControl>
            <FormControl.Label>Repository name</FormControl.Label>
            <div style={{ display: 'flex', gap: '8px', alignItems: 'flex-start' }}>
              <TextInput
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                block
              />
              <Button
                onClick={handleRename}
                disabled={renaming || !newName.trim() || newName === repo}
              >
                {renaming ? 'Renaming...' : 'Rename'}
              </Button>
            </div>
          </FormControl>
          {renameStatus && (
            <Flash variant={renameStatus.type} style={{ marginTop: '8px' }}>
              {renameStatus.msg}
            </Flash>
          )}
        </div>

        {/* Description */}
        <div style={{ marginBottom: '24px' }}>
          <FormControl>
            <FormControl.Label>Description</FormControl.Label>
            <div style={{ display: 'flex', gap: '8px', alignItems: 'flex-start' }}>
              <TextInput
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Short description of this repository"
                block
              />
              <Button onClick={handleSaveDescription} disabled={savingDesc}>
                {savingDesc ? 'Saving...' : 'Save'}
              </Button>
            </div>
          </FormControl>
          {descStatus && (
            <Flash variant={descStatus.type} style={{ marginTop: '8px' }}>
              {descStatus.msg}
            </Flash>
          )}
        </div>

        {/* Clone URL */}
        <div style={{ marginBottom: '32px' }}>
          <FormControl>
            <FormControl.Label>Remote URL</FormControl.Label>
            <div style={{ display: 'flex', gap: '4px' }}>
              <TextInput
                value={`forge clone ${cloneUrl}`}
                readOnly
                block
                monospace
                size="small"
              />
              <Button size="small" onClick={handleCopyClone}>
                {copied ? 'Copied!' : <CopyIcon size={16} />}
              </Button>
            </div>
          </FormControl>
        </div>

        {/* Default Branch */}
        <div className="forge-card" style={{ marginBottom: '24px' }}>
          <div className="forge-card-header"><h3>Default Branch</h3></div>
          <div style={{ padding: '16px' }}>
            <p style={{ color: 'var(--fg-muted)', marginBottom: '12px' }}>The default branch is the base for new pull requests and code browsing.</p>
            <select value={defaultBranch} onChange={e => setDefaultBranch(e.target.value)} style={{ padding: '6px 12px', borderRadius: '6px', border: '1px solid var(--border-default)', backgroundColor: 'var(--bg-default)', color: 'var(--fg-default)' }}>
              {branches.map(b => <option key={b.name} value={b.name}>{b.name}</option>)}
            </select>
          </div>
        </div>

        {/* Lock Policy */}
        <div className="forge-card" style={{ marginBottom: '24px' }}>
          <div className="forge-card-header"><h3>File Lock Policy</h3></div>
          <div style={{ padding: '16px' }}>
            <p style={{ color: 'var(--fg-muted)', marginBottom: '12px' }}>Binary files matching these patterns require exclusive locks before editing.</p>
            <div style={{ fontFamily: 'monospace', fontSize: '13px', padding: '12px', backgroundColor: 'var(--bg-inset)', borderRadius: '6px', border: '1px solid var(--border-muted)' }}>
              *.uasset<br/>*.umap<br/>*.uexp<br/>*.ubulk
            </div>
          </div>
        </div>

        {/* Danger Zone */}
        <div
          style={{
            border: '1px solid var(--fg-danger, #da3633)',
            borderRadius: '6px',
            padding: '16px',
          }}
        >
          <h3 style={{
            fontSize: '16px',
            fontWeight: 600,
            color: 'var(--fg-danger, #da3633)',
            marginBottom: '16px',
            display: 'flex',
            alignItems: 'center',
            gap: '8px',
          }}>
            <AlertIcon size={16} />
            Danger Zone
          </h3>

          {!showDeleteDialog ? (
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              <div>
                <div style={{ fontWeight: 600, fontSize: '14px', color: 'var(--fg-default)' }}>
                  Delete this repository
                </div>
                <div style={{ fontSize: '12px', color: 'var(--fg-muted)' }}>
                  Once you delete a repository, there is no going back.
                </div>
              </div>
              <Button variant="danger" onClick={() => setShowDeleteDialog(true)}>
                Delete this repository
              </Button>
            </div>
          ) : (
            <div>
              <Flash variant="danger" style={{ marginBottom: '16px' }}>
                This action <strong>cannot</strong> be undone. This will permanently delete the{' '}
                <strong>{repo}</strong> repository and all of its data.
              </Flash>

              <FormControl>
                <FormControl.Label>
                  Please type <strong>{repo}</strong> to confirm.
                </FormControl.Label>
                <TextInput
                  value={deleteConfirm}
                  onChange={(e) => setDeleteConfirm(e.target.value)}
                  block
                  placeholder={repo}
                />
              </FormControl>

              {deleteError && (
                <Flash variant="danger" style={{ marginTop: '8px' }}>
                  {deleteError}
                </Flash>
              )}

              <div style={{ display: 'flex', gap: '8px', marginTop: '12px' }}>
                <Button variant="danger" onClick={handleDelete} disabled={deleteConfirm !== repo || deleting}>
                  {deleting ? 'Deleting...' : 'I understand, delete this repository'}
                </Button>
                <Button onClick={() => { setShowDeleteDialog(false); setDeleteConfirm(''); }}>
                  Cancel
                </Button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
