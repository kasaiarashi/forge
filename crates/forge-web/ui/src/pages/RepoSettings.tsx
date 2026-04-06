import { useEffect, useState } from 'react';
import { useParams, Link, useNavigate } from 'react-router-dom';
import {
  TextInput,
  Button,
  FormControl,
  Flash,
  Spinner,
  UnderlineNav,
} from '@primer/react';
import {
  GearIcon,
  CodeIcon,
  GitCommitIcon,
  LockIcon,
  RepoIcon,
  AlertIcon,
  CopyIcon,
} from '@primer/octicons-react';
import type { RepoInfo, Branch } from '../api';
import api, { copyToClipboard } from '../api';

export default function RepoSettings() {
  const { repo = '' } = useParams();
  const navigate = useNavigate();
  const encRepo = encodeURIComponent(repo);

  const [, setRepoInfo] = useState<RepoInfo | null>(null);
  const [defaultBranch, setDefaultBranch] = useState('main');
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
      .then(([repos, branches]) => {
        const info = repos.find((r) => r.name === repo);
        if (info) {
          setRepoInfo(info);
          setNewName(info.name);
          setDescription(info.description || '');
        }
        const main = branches.find((b: Branch) => b.name === 'main') || branches[0];
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

  const cloneUrl = `${window.location.protocol}//${window.location.hostname}:9876`;

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

      {/* Repository tabs */}
      <UnderlineNav aria-label="Repository">
        <UnderlineNav.Item
          as={Link}
          to={`/${encRepo}/tree/${encodeURIComponent(defaultBranch)}`}
          icon={CodeIcon}
        >
          Code
        </UnderlineNav.Item>
        <UnderlineNav.Item
          as={Link}
          to={`/${encRepo}/commits/${encodeURIComponent(defaultBranch)}`}
          icon={GitCommitIcon}
        >
          Commits
        </UnderlineNav.Item>
        <UnderlineNav.Item as={Link} to={`/${encRepo}/locks`} icon={LockIcon}>
          Locks
        </UnderlineNav.Item>
        <UnderlineNav.Item as={Link} to={`/${encRepo}/settings`} aria-current="page" icon={GearIcon}>
          Settings
        </UnderlineNav.Item>
      </UnderlineNav>

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
