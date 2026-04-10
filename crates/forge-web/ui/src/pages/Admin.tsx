import { useEffect, useState } from 'react';
import type { FC } from 'react';
import { Link } from 'react-router-dom';
import {
  Spinner,
  Flash,
  Label,
  Button,
  TextInput,
  FormControl,
} from '@primer/react';
import {
  GearIcon,
  ServerIcon,
  ClockIcon,
  GitBranchIcon,
  LockIcon,
  GitCommitIcon,
  DatabaseIcon,
  SignInIcon,
  ShieldLockIcon,
  PeopleIcon,
  TrashIcon,
} from '@primer/octicons-react';
import type { ServerInfo, UserSummary } from '../api';
import api from '../api';
import { useAuth } from '../context/AuthContext';

function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  const parts: string[] = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  parts.push(`${mins}m`);
  return parts.join(' ');
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

interface StatCardProps {
  icon: FC<{ size?: number; className?: string }>;
  label: string;
  value: string;
  color?: string;
}

function StatCard({ icon: Icon, label, value, color = 'var(--fg-default)' }: StatCardProps) {
  return (
    <div className="stat-card">
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '4px' }}>
        <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><Icon size={16} /></span>
        <span style={{ fontSize: '12px', color: 'var(--fg-muted)', fontWeight: 600, textTransform: 'uppercase' }}>
          {label}
        </span>
      </div>
      <span style={{ fontSize: '28px', fontWeight: 'bold', color }}>
        {value}
      </span>
    </div>
  );
}

export default function Admin() {
  const { user, loading: authLoading } = useAuth();
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  // User management state
  const [users, setUsers] = useState<UserSummary[]>([]);
  const [usersLoading, setUsersLoading] = useState(true);
  const [usersError, setUsersError] = useState('');
  const [showCreateUser, setShowCreateUser] = useState(false);
  const [newUser, setNewUser] = useState({ username: '', email: '', display_name: '', password: '', is_server_admin: false });
  const [creatingUser, setCreatingUser] = useState(false);
  const [createUserError, setCreateUserError] = useState('');
  const [deleteConfirmId, setDeleteConfirmId] = useState<number | null>(null);

  useEffect(() => {
    if (authLoading) return;
    if (!user || !user.is_admin) {
      setLoading(false);
      return;
    }
    api
      .getServerInfo()
      .then(setInfo)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));

    api.listUsers()
      .then(setUsers)
      .catch((e) => setUsersError(e.message))
      .finally(() => setUsersLoading(false));
  }, [user, authLoading]);

  const handleCreateUser = async () => {
    if (!newUser.username.trim() || !newUser.email.trim() || !newUser.password) return;
    setCreatingUser(true);
    setCreateUserError('');
    try {
      const res = await api.createUser(newUser);
      setUsers((prev) => [...prev, res.user]);
      setNewUser({ username: '', email: '', display_name: '', password: '', is_server_admin: false });
      setShowCreateUser(false);
    } catch (e) {
      setCreateUserError(e instanceof Error ? e.message : 'Failed to create user');
    } finally {
      setCreatingUser(false);
    }
  };

  const handleDeleteUser = async (id: number) => {
    try {
      await api.deleteUser(id);
      setUsers((prev) => prev.filter((u) => u.id !== id));
      setDeleteConfirmId(null);
    } catch (e) {
      setUsersError(e instanceof Error ? e.message : 'Failed to delete user');
    }
  };

  if (authLoading || loading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
        <Spinner size="large" />
      </div>
    );
  }

  if (!user) {
    return (
      <div style={{
        display: 'flex',
        justifyContent: 'center',
        padding: '48px 0',
      }}>
        <div className="forge-card" style={{
          padding: '48px',
          textAlign: 'center',
          maxWidth: 400,
        }}>
          <div style={{ color: 'var(--fg-muted)', marginBottom: '16px', display: 'flex', justifyContent: 'center' }}>
            <SignInIcon size={40} />
          </div>
          <h2 style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-default)', margin: '0 0 8px 0' }}>
            Sign in required
          </h2>
          <p style={{ color: 'var(--fg-muted)', fontSize: '14px', margin: '0 0 16px 0' }}>
            You need to sign in to access the admin panel.
          </p>
          <Button as={Link} to="/login" variant="primary">
            Sign in
          </Button>
        </div>
      </div>
    );
  }

  if (!user.is_admin) {
    return (
      <div style={{
        display: 'flex',
        justifyContent: 'center',
        padding: '48px 0',
      }}>
        <div className="forge-card" style={{
          padding: '48px',
          textAlign: 'center',
          maxWidth: 400,
        }}>
          <div style={{ color: 'var(--fg-danger)', marginBottom: '16px', display: 'flex', justifyContent: 'center' }}>
            <ShieldLockIcon size={40} />
          </div>
          <h2 style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-default)', margin: '0 0 8px 0' }}>
            Access denied
          </h2>
          <p style={{ color: 'var(--fg-muted)', fontSize: '14px', margin: '0 0 16px 0' }}>
            You do not have admin privileges. Contact your server administrator.
          </p>
          <Button as={Link} to="/" variant="default">
            Back to repositories
          </Button>
        </div>
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

  if (!info) return null;

  return (
    <div>
      {/* Page header */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        gap: '8px',
        marginBottom: '24px',
        paddingBottom: '16px',
        borderBottom: '1px solid var(--border-default)',
      }}>
        <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><GearIcon size={24} /></span>
        <h2 style={{ fontSize: '20px', fontWeight: 600, margin: 0 }}>
          Server Administration
        </h2>
        <Label variant="accent">Admin</Label>
      </div>

      {/* Server info section */}
      <div style={{ marginBottom: '24px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
          <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><ServerIcon size={20} /></span>
          <h3 style={{ fontSize: '16px', fontWeight: 600, margin: 0 }}>
            Server Information
          </h3>
        </div>

        <div style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))',
          gap: '16px',
        }}>
          <StatCard icon={ServerIcon} label="Version" value={info.version} />
          <StatCard icon={ClockIcon} label="Uptime" value={formatUptime(info.uptime_secs)} />
          <StatCard icon={GitBranchIcon} label="Branches" value={String(info.branches.length)} color="var(--fg-accent)" />
          <StatCard
            icon={LockIcon}
            label="Active Locks"
            value={String(info.active_locks)}
            color={info.active_locks > 0 ? 'var(--fg-warning)' : 'var(--fg-success)'}
          />
          <StatCard icon={GitCommitIcon} label="Total Objects" value={String(info.total_objects)} />
          <StatCard icon={DatabaseIcon} label="Storage" value={formatBytes(info.total_size_bytes)} />
        </div>
      </div>

      {/* Settings section */}
      <div style={{ marginBottom: '24px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
          <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><GearIcon size={20} /></span>
          <h3 style={{ fontSize: '16px', fontWeight: 600, margin: 0 }}>
            Settings
          </h3>
        </div>

        <div className="forge-card">
          <div style={{
            padding: '16px',
            borderBottom: '1px solid var(--border-muted)',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}>
            <div>
              <div style={{ fontWeight: 600, fontSize: '14px' }}>Server URL</div>
              <div style={{ color: 'var(--fg-muted)', fontSize: '12px' }}>
                The address clients connect to.
              </div>
            </div>
            <code className="text-mono" style={{
              fontSize: '14px',
              color: 'var(--fg-muted)',
              background: 'var(--bg-subtle)',
              padding: '4px 8px',
              borderRadius: '6px',
              border: '1px solid var(--border-default)',
            }}>
              {window.location.origin}
            </code>
          </div>

          <div style={{
            padding: '16px',
            borderBottom: '1px solid var(--border-muted)',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}>
            <div>
              <div style={{ fontWeight: 600, fontSize: '14px' }}>Binary storage</div>
              <div style={{ color: 'var(--fg-muted)', fontSize: '12px' }}>
                Total size of all stored objects.
              </div>
            </div>
            <Label variant="secondary">{formatBytes(info.total_size_bytes)}</Label>
          </div>

          <div style={{
            padding: '16px',
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
          }}>
            <div>
              <div style={{ fontWeight: 600, fontSize: '14px' }}>File locking</div>
              <div style={{ color: 'var(--fg-muted)', fontSize: '12px' }}>
                Prevents concurrent edits to binary assets.
              </div>
            </div>
            <Label variant="success">Enabled</Label>
          </div>
        </div>
      </div>

      {/* Users section */}
      <div style={{ marginBottom: '24px' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: '16px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><PeopleIcon size={20} /></span>
            <h3 style={{ fontSize: '16px', fontWeight: 600, margin: 0 }}>Users</h3>
            {!usersLoading && <Label size="small" variant="secondary">{users.length}</Label>}
          </div>
          <Button size="small" variant="primary" onClick={() => setShowCreateUser(!showCreateUser)}>
            {showCreateUser ? 'Cancel' : 'New user'}
          </Button>
        </div>

        {/* Create user form */}
        {showCreateUser && (
          <div className="forge-card" style={{ marginBottom: '16px' }}>
            <div className="forge-card-header"><h4 style={{ margin: 0, fontSize: '14px' }}>Create user</h4></div>
            <div style={{ padding: '16px' }}>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px', marginBottom: '12px' }}>
                <FormControl>
                  <FormControl.Label>Username</FormControl.Label>
                  <TextInput
                    value={newUser.username}
                    onChange={(e) => setNewUser({ ...newUser, username: e.target.value })}
                    placeholder="alice"
                    size="small"
                    block
                  />
                </FormControl>
                <FormControl>
                  <FormControl.Label>Email</FormControl.Label>
                  <TextInput
                    value={newUser.email}
                    onChange={(e) => setNewUser({ ...newUser, email: e.target.value })}
                    placeholder="alice@example.com"
                    size="small"
                    block
                  />
                </FormControl>
                <FormControl>
                  <FormControl.Label>Display name</FormControl.Label>
                  <TextInput
                    value={newUser.display_name}
                    onChange={(e) => setNewUser({ ...newUser, display_name: e.target.value })}
                    placeholder="Alice Smith"
                    size="small"
                    block
                  />
                </FormControl>
                <FormControl>
                  <FormControl.Label>Password</FormControl.Label>
                  <TextInput
                    type="password"
                    value={newUser.password}
                    onChange={(e) => setNewUser({ ...newUser, password: e.target.value })}
                    placeholder="Initial password"
                    size="small"
                    block
                  />
                </FormControl>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                <label style={{ display: 'flex', alignItems: 'center', gap: '6px', cursor: 'pointer', fontSize: '13px' }}>
                  <input
                    type="checkbox"
                    checked={newUser.is_server_admin}
                    onChange={(e) => setNewUser({ ...newUser, is_server_admin: e.target.checked })}
                  />
                  Server administrator
                </label>
                <Button
                  variant="primary"
                  size="small"
                  onClick={handleCreateUser}
                  disabled={creatingUser || !newUser.username.trim() || !newUser.email.trim() || !newUser.password}
                >
                  {creatingUser ? 'Creating...' : 'Create user'}
                </Button>
              </div>
              {createUserError && <Flash variant="danger" style={{ marginTop: '8px' }}>{createUserError}</Flash>}
            </div>
          </div>
        )}

        {/* User list */}
        {usersLoading ? (
          <Spinner size="small" />
        ) : usersError ? (
          <Flash variant="danger">{usersError}</Flash>
        ) : (
          <div className="forge-card">
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead>
                <tr style={{ borderBottom: '1px solid var(--border-muted)' }}>
                  <th style={{ textAlign: 'left', padding: '10px 16px', fontSize: '12px', color: 'var(--fg-muted)', fontWeight: 600 }}>Username</th>
                  <th style={{ textAlign: 'left', padding: '10px 16px', fontSize: '12px', color: 'var(--fg-muted)', fontWeight: 600 }}>Email</th>
                  <th style={{ textAlign: 'left', padding: '10px 16px', fontSize: '12px', color: 'var(--fg-muted)', fontWeight: 600 }}>Display Name</th>
                  <th style={{ textAlign: 'left', padding: '10px 16px', fontSize: '12px', color: 'var(--fg-muted)', fontWeight: 600 }}>Role</th>
                  <th style={{ width: '40px' }}></th>
                </tr>
              </thead>
              <tbody>
                {users.map((u) => (
                  <tr key={u.id} style={{ borderBottom: '1px solid var(--border-muted)' }}>
                    <td style={{ padding: '10px 16px', fontWeight: 600, fontSize: '14px' }}>{u.username}</td>
                    <td style={{ padding: '10px 16px', fontSize: '14px', color: 'var(--fg-muted)' }}>{u.email}</td>
                    <td style={{ padding: '10px 16px', fontSize: '14px' }}>{u.display_name}</td>
                    <td style={{ padding: '10px 16px' }}>
                      <Label size="small" variant={u.is_server_admin ? 'accent' : 'secondary'}>
                        {u.is_server_admin ? 'Admin' : 'User'}
                      </Label>
                    </td>
                    <td style={{ padding: '10px 16px', textAlign: 'right' }}>
                      {deleteConfirmId === u.id ? (
                        <div style={{ display: 'flex', gap: '4px' }}>
                          <Button variant="danger" size="small" onClick={() => handleDeleteUser(u.id)}>
                            Confirm
                          </Button>
                          <Button size="small" onClick={() => setDeleteConfirmId(null)}>
                            Cancel
                          </Button>
                        </div>
                      ) : (
                        <Button variant="danger" size="small" onClick={() => setDeleteConfirmId(u.id)}>
                          <TrashIcon size={14} />
                        </Button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
