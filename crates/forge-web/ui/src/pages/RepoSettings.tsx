import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import {
  TextInput,
  Textarea,
  Button,
  FormControl,
  Flash,
  Spinner,
} from '@primer/react';
import {
  AlertIcon,
  CopyIcon,
  PeopleIcon,
  TrashIcon,
} from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import type { RepoInfo, Branch, RepoMember } from '../api';
import api, { copyToClipboard } from '../api';

export default function RepoSettings() {
  const repo = useRepoParam();
  const navigate = useNavigate();

  const [repoInfo, setRepoInfo] = useState<RepoInfo | null>(null);
  const [defaultBranch, setDefaultBranch] = useState('main');
  const [branches, setBranches] = useState<Branch[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  // General settings state
  const [newName, setNewName] = useState('');
  const [description, setDescription] = useState('');
  const [visibility, setVisibility] = useState('private');
  const [renameStatus, setRenameStatus] = useState<{ type: 'success' | 'danger'; msg: string } | null>(null);
  const [descStatus, setDescStatus] = useState<{ type: 'success' | 'danger'; msg: string } | null>(null);
  const [visStatus, setVisStatus] = useState<{ type: 'success' | 'danger'; msg: string } | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [savingDesc, setSavingDesc] = useState(false);
  const [savingVis, setSavingVis] = useState(false);

  // Delete state
  const [deleteConfirm, setDeleteConfirm] = useState('');
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState('');
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);

  // Clone URL copy
  const [copied, setCopied] = useState(false);

  // Collaborators state
  const [members, setMembers] = useState<RepoMember[]>([]);
  const [membersLoading, setMembersLoading] = useState(true);
  const [membersError, setMembersError] = useState('');
  const [newUsername, setNewUsername] = useState('');
  const [newRole, setNewRole] = useState('write');
  const [adding, setAdding] = useState(false);
  const [addError, setAddError] = useState('');

  useEffect(() => {
    setLoading(true);
    Promise.all([api.listRepos(), api.listBranches(repo)])
      .then(([repos, br]) => {
        const info = repos.find((r) => r.name === repo);
        if (info) {
          setRepoInfo(info);
          setNewName(info.name);
          setDescription(info.description || '');
          setVisibility(info.visibility || 'private');
        }
        setBranches(br);
        const main = br.find((b: Branch) => b.name === 'main') || br[0];
        if (main) setDefaultBranch(main.name);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));

    // Fetch members separately so a 403 doesn't break the rest of settings
    api.listRepoMembers(repo)
      .then(setMembers)
      .catch((e) => setMembersError(e.message))
      .finally(() => setMembersLoading(false));
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

  const handleVisibilityChange = async (newVis: string) => {
    if (newVis === visibility) return;
    setSavingVis(true);
    setVisStatus(null);
    try {
      await api.updateRepo(repo, { visibility: newVis });
      setVisibility(newVis);
      setVisStatus({ type: 'success', msg: `Repository is now ${newVis}.` });
    } catch (e) {
      setVisStatus({ type: 'danger', msg: e instanceof Error ? e.message : 'Failed to update visibility' });
    } finally {
      setSavingVis(false);
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

  const handleAddMember = async () => {
    if (!newUsername.trim()) return;
    setAdding(true);
    setAddError('');
    try {
      const user = await api.lookupUser(newUsername.trim());
      await api.addRepoMember(repo, user.id, newRole);
      const updated = await api.listRepoMembers(repo);
      setMembers(updated);
      setNewUsername('');
      setNewRole('write');
    } catch (e) {
      setAddError(e instanceof Error ? e.message : 'Failed to add collaborator');
    } finally {
      setAdding(false);
    }
  };

  const handleChangeRole = async (userId: number, role: string) => {
    try {
      await api.addRepoMember(repo, userId, role);
      const updated = await api.listRepoMembers(repo);
      setMembers(updated);
    } catch (e) {
      setMembersError(e instanceof Error ? e.message : 'Failed to update role');
    }
  };

  const handleRemoveMember = async (userId: number) => {
    try {
      await api.removeRepoMember(repo, userId);
      setMembers((prev) => prev.filter((m) => m.user.id !== userId));
    } catch (e) {
      setMembersError(e instanceof Error ? e.message : 'Failed to remove collaborator');
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

  return (
    <div>
      <RepoHeader repo={repo} currentTab="settings" activeBranch={defaultBranch} visibility={repoInfo?.visibility} />
      <div style={{ maxWidth: '1280px', margin: '0 auto', padding: '0 var(--space-6)' }}>
        <div style={{ marginTop: '24px', maxWidth: '720px' }}>
          {/* General Settings */}
          <div className="forge-card" style={{ marginBottom: '24px' }}>
            <div className="forge-card-header"><h3>General Configuration</h3></div>
            <div style={{ padding: '16px', display: 'flex', flexDirection: 'column', gap: '24px' }}>

              {/* Repository name */}
              <div>
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
              <div>
                <FormControl>
                  <FormControl.Label>Description</FormControl.Label>
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'flex-start' }}>
                    <Textarea
                      value={description}
                      onChange={(e) => setDescription(e.target.value)}
                      placeholder="Short description of this repository"
                      block
                      resize="vertical"
                      style={{ minHeight: '100px' }}
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
              <div>
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

            </div>
          </div>

          {/* Visibility */}
          <div className="forge-card" style={{ marginBottom: '24px' }}>
            <div className="forge-card-header"><h3>Visibility</h3></div>
            <div style={{ padding: '16px' }}>
              <p style={{ color: 'var(--fg-muted)', marginBottom: '12px' }}>
                Public repositories are visible to anyone. Private repositories require explicit access.
              </p>
              <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                <select
                  value={visibility}
                  onChange={(e) => handleVisibilityChange(e.target.value)}
                  disabled={savingVis}
                  style={{ padding: '6px 12px', borderRadius: '6px', border: '1px solid var(--border-default)', backgroundColor: 'var(--bg-default)', color: 'var(--fg-default)' }}
                >
                  <option value="private">Private</option>
                  <option value="public">Public</option>
                </select>
                {savingVis && <Spinner size="small" />}
              </div>
              {visStatus && (
                <Flash variant={visStatus.type} style={{ marginTop: '8px' }}>{visStatus.msg}</Flash>
              )}
            </div>
          </div>

          {/* Default Branch */}
          <div className="forge-card" style={{ marginBottom: '24px' }}>
            <div className="forge-card-header"><h3>Default Branch</h3></div>
            <div style={{ padding: '16px' }}>
              <p style={{ color: 'var(--fg-muted)', marginBottom: '12px' }}>The default branch is the base for new pull requests and code browsing.</p>
              <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                <select value={defaultBranch} onChange={e => setDefaultBranch(e.target.value)} style={{ padding: '6px 12px', borderRadius: '6px', border: '1px solid var(--border-default)', backgroundColor: 'var(--bg-default)', color: 'var(--fg-default)' }}>
                  {branches.map(b => <option key={b.name} value={b.name}>{b.name}</option>)}
                </select>
                <Button size="small" onClick={async () => {
                  try {
                    await api.updateRepo(repo, { default_branch: defaultBranch });
                  } catch { }
                }}>
                  Save
                </Button>
              </div>
            </div>
          </div>

          {/* Lock Policy */}
          <div className="forge-card" style={{ marginBottom: '24px' }}>
            <div className="forge-card-header"><h3>File Lock Policy</h3></div>
            <div style={{ padding: '16px' }}>
              <p style={{ color: 'var(--fg-muted)', marginBottom: '12px' }}>Binary files matching these patterns require exclusive locks before editing.</p>
              <div style={{ fontFamily: 'monospace', fontSize: '13px', padding: '12px', backgroundColor: 'var(--bg-inset)', borderRadius: '6px', border: '1px solid var(--border-muted)' }}>
                *.uasset<br />*.umap<br />*.uexp<br />*.ubulk
              </div>
            </div>
          </div>

          {/* Collaborators */}
          <div className="forge-card" style={{ marginBottom: '24px' }}>
            <div className="forge-card-header" style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <PeopleIcon size={16} />
              <h3>Collaborators</h3>
            </div>
            <div style={{ padding: '16px' }}>
              {membersLoading ? (
                <Spinner size="small" />
              ) : membersError ? (
                <Flash variant="danger">{membersError}</Flash>
              ) : (
                <>
                  {members.length === 0 ? (
                    <p style={{ color: 'var(--fg-muted)', marginBottom: '16px' }}>
                      No collaborators yet. Add users to grant them access to this repository.
                    </p>
                  ) : (
                    <table style={{ width: '100%', borderCollapse: 'collapse', marginBottom: '16px' }}>
                      <thead>
                        <tr style={{ borderBottom: '1px solid var(--border-muted)' }}>
                          <th style={{ textAlign: 'left', padding: '8px 0', fontSize: '12px', color: 'var(--fg-muted)', fontWeight: 600 }}>User</th>
                          <th style={{ textAlign: 'left', padding: '8px 0', fontSize: '12px', color: 'var(--fg-muted)', fontWeight: 600 }}>Role</th>
                          <th style={{ width: '40px' }}></th>
                        </tr>
                      </thead>
                      <tbody>
                        {members.map((m) => (
                          <tr key={m.user.id} style={{ borderBottom: '1px solid var(--border-muted)' }}>
                            <td style={{ padding: '8px 0' }}>
                              <span style={{ fontWeight: 600 }}>{m.user.username}</span>
                              {m.user.display_name && (
                                <span style={{ color: 'var(--fg-muted)', marginLeft: '8px' }}>{m.user.display_name}</span>
                              )}
                            </td>
                            <td style={{ padding: '8px 0' }}>
                              <select
                                value={m.role}
                                onChange={(e) => handleChangeRole(m.user.id, e.target.value)}
                                style={{ padding: '4px 8px', borderRadius: '6px', border: '1px solid var(--border-default)', backgroundColor: 'var(--bg-default)', color: 'var(--fg-default)', fontSize: '13px' }}
                              >
                                <option value="read">Read</option>
                                <option value="write">Write</option>
                                <option value="admin">Admin</option>
                              </select>
                            </td>
                            <td style={{ padding: '8px 0', textAlign: 'right' }}>
                              <Button variant="danger" size="small" onClick={() => handleRemoveMember(m.user.id)}>
                                <TrashIcon size={14} />
                              </Button>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  )}

                  {/* Add collaborator form */}
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'flex-end' }}>
                    <FormControl>
                      <FormControl.Label>Username</FormControl.Label>
                      <TextInput
                        value={newUsername}
                        onChange={(e) => setNewUsername(e.target.value)}
                        placeholder="Enter username"
                        size="small"
                      />
                    </FormControl>
                    <FormControl>
                      <FormControl.Label>Role</FormControl.Label>
                      <select
                        value={newRole}
                        onChange={(e) => setNewRole(e.target.value)}
                        style={{ padding: '6px 12px', borderRadius: '6px', border: '1px solid var(--border-default)', backgroundColor: 'var(--bg-default)', color: 'var(--fg-default)' }}
                      >
                        <option value="read">Read</option>
                        <option value="write">Write</option>
                        <option value="admin">Admin</option>
                      </select>
                    </FormControl>
                    <Button onClick={handleAddMember} disabled={adding || !newUsername.trim()} size="small">
                      {adding ? 'Adding...' : 'Add'}
                    </Button>
                  </div>
                  {addError && (
                    <Flash variant="danger" style={{ marginTop: '8px' }}>{addError}</Flash>
                  )}
                </>
              )}
            </div>
          </div>

          {/* Danger Zone */}
          <div className="forge-card" style={{ marginBottom: '24px', border: '1px solid var(--fg-danger, #da3633)' }}>
            <div className="forge-card-header" style={{ borderBottomColor: 'var(--fg-danger, #da3633)', backgroundColor: 'rgba(218,54,51,0.05)' }}>
              <h3 style={{ color: 'var(--fg-danger, #da3633)', display: 'flex', alignItems: 'center', gap: '8px' }}>
                <AlertIcon size={16} /> Danger Zone
              </h3>
            </div>
            <div style={{ padding: '16px' }}>

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
      </div>
    </div>
  );
}
