import { useEffect, useState } from 'react';
import {
  TextInput,
  Button,
  FormControl,
  Flash,
  Spinner,
  Label,
} from '@primer/react';
import {
  KeyIcon,
  DeviceMobileIcon,
  TrashIcon,
  CopyIcon,
  ShieldLockIcon,
} from '@primer/octicons-react';
import type { PatInfo, SessionInfo } from '../api';
import api, { copyToClipboard } from '../api';

function timeAgo(ts: number): string {
  if (!ts) return 'Never';
  const secs = Math.floor(Date.now() / 1000) - ts;
  if (secs < 60) return 'just now';
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

function formatDate(ts: number): string {
  if (!ts) return 'Never';
  return new Date(ts * 1000).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

const ALL_SCOPES = ['repo:read', 'repo:write', 'repo:admin', 'user:admin'];

export default function AccountSettings() {
  // PATs
  const [tokens, setTokens] = useState<PatInfo[]>([]);
  const [tokensLoading, setTokensLoading] = useState(true);
  const [tokensError, setTokensError] = useState('');
  const [newTokenName, setNewTokenName] = useState('');
  const [newTokenScopes, setNewTokenScopes] = useState<string[]>(['repo:read', 'repo:write']);
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState('');
  const [newPlaintext, setNewPlaintext] = useState('');
  const [copied, setCopied] = useState(false);

  // Sessions
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [sessionsLoading, setSessionsLoading] = useState(true);
  const [sessionsError, setSessionsError] = useState('');

  useEffect(() => {
    api.listTokens()
      .then(setTokens)
      .catch((e) => setTokensError(e.message))
      .finally(() => setTokensLoading(false));

    api.listSessions()
      .then(setSessions)
      .catch((e) => setSessionsError(e.message))
      .finally(() => setSessionsLoading(false));
  }, []);

  const handleCreateToken = async () => {
    if (!newTokenName.trim() || newTokenScopes.length === 0) return;
    setCreating(true);
    setCreateError('');
    setNewPlaintext('');
    try {
      const res = await api.createToken(newTokenName.trim(), newTokenScopes);
      setNewPlaintext(res.plaintext_token);
      setTokens((prev) => [res.pat, ...prev]);
      setNewTokenName('');
      setNewTokenScopes(['repo:read', 'repo:write']);
    } catch (e) {
      setCreateError(e instanceof Error ? e.message : 'Failed to create token');
    } finally {
      setCreating(false);
    }
  };

  const handleDeleteToken = async (id: number) => {
    try {
      await api.deleteToken(id);
      setTokens((prev) => prev.filter((t) => t.id !== id));
    } catch (e) {
      setTokensError(e instanceof Error ? e.message : 'Failed to revoke token');
    }
  };

  const handleDeleteSession = async (id: number) => {
    try {
      await api.deleteSession(id);
      setSessions((prev) => prev.filter((s) => s.id !== id));
    } catch (e) {
      setSessionsError(e instanceof Error ? e.message : 'Failed to revoke session');
    }
  };

  const toggleScope = (scope: string) => {
    setNewTokenScopes((prev) =>
      prev.includes(scope) ? prev.filter((s) => s !== scope) : [...prev, scope],
    );
  };

  const handleCopyToken = () => {
    copyToClipboard(newPlaintext);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div>
      {/* Page header */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: '8px',
        marginBottom: '24px', paddingBottom: '16px',
        borderBottom: '1px solid var(--border-default)',
      }}>
        <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><ShieldLockIcon size={24} /></span>
        <h2 style={{ fontSize: '20px', fontWeight: 600, margin: 0 }}>Account Settings</h2>
      </div>

      <div style={{ maxWidth: '720px' }}>
        {/* ── Personal Access Tokens ────────────────────────────── */}
        <div style={{ marginBottom: '32px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
            <KeyIcon size={20} />
            <h3 style={{ fontSize: '16px', fontWeight: 600, margin: 0 }}>Personal Access Tokens</h3>
          </div>

          {/* New token plaintext banner */}
          {newPlaintext && (
            <Flash variant="success" style={{ marginBottom: '16px' }}>
              <div style={{ marginBottom: '8px' }}>
                <strong>Token created.</strong> Copy it now — you won't be able to see it again.
              </div>
              <div style={{ display: 'flex', gap: '4px' }}>
                <TextInput value={newPlaintext} readOnly block monospace size="small" />
                <Button size="small" onClick={handleCopyToken}>
                  {copied ? 'Copied!' : <CopyIcon size={16} />}
                </Button>
              </div>
            </Flash>
          )}

          {/* Create token form */}
          <div className="forge-card" style={{ marginBottom: '16px' }}>
            <div className="forge-card-header"><h4 style={{ margin: 0, fontSize: '14px' }}>Generate new token</h4></div>
            <div style={{ padding: '16px' }}>
              <div style={{ display: 'flex', gap: '8px', marginBottom: '12px', alignItems: 'flex-end' }}>
                <FormControl>
                  <FormControl.Label>Name</FormControl.Label>
                  <TextInput
                    value={newTokenName}
                    onChange={(e) => setNewTokenName(e.target.value)}
                    placeholder="e.g. ci-pipeline"
                    size="small"
                  />
                </FormControl>
                <Button onClick={handleCreateToken} disabled={creating || !newTokenName.trim() || newTokenScopes.length === 0} size="small" variant="primary">
                  {creating ? 'Creating...' : 'Generate token'}
                </Button>
              </div>
              <div>
                <span style={{ fontSize: '12px', fontWeight: 600, color: 'var(--fg-muted)' }}>Scopes</span>
                <div style={{ display: 'flex', gap: '8px', marginTop: '4px', flexWrap: 'wrap' }}>
                  {ALL_SCOPES.map((scope) => (
                    <label key={scope} style={{ display: 'flex', alignItems: 'center', gap: '4px', cursor: 'pointer', fontSize: '13px' }}>
                      <input
                        type="checkbox"
                        checked={newTokenScopes.includes(scope)}
                        onChange={() => toggleScope(scope)}
                      />
                      <code style={{ fontSize: '12px' }}>{scope}</code>
                    </label>
                  ))}
                </div>
              </div>
              {createError && <Flash variant="danger" style={{ marginTop: '8px' }}>{createError}</Flash>}
            </div>
          </div>

          {/* Token list */}
          {tokensLoading ? (
            <Spinner size="small" />
          ) : tokensError ? (
            <Flash variant="danger">{tokensError}</Flash>
          ) : tokens.length === 0 ? (
            <p style={{ color: 'var(--fg-muted)', fontSize: '14px' }}>No personal access tokens.</p>
          ) : (
            <div className="forge-card">
              {tokens.map((t, i) => (
                <div
                  key={t.id}
                  style={{
                    padding: '12px 16px',
                    display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                    borderBottom: i < tokens.length - 1 ? '1px solid var(--border-muted)' : undefined,
                  }}
                >
                  <div>
                    <div style={{ fontWeight: 600, fontSize: '14px' }}>{t.name}</div>
                    <div style={{ fontSize: '12px', color: 'var(--fg-muted)', display: 'flex', gap: '8px', marginTop: '2px', flexWrap: 'wrap' }}>
                      {t.scopes.map((s) => (
                        <Label key={s} size="small" variant="secondary">{s}</Label>
                      ))}
                      <span>Created {formatDate(t.created_at)}</span>
                      <span>Last used {timeAgo(t.last_used_at)}</span>
                      {t.expires_at > 0 && <span>Expires {formatDate(t.expires_at)}</span>}
                    </div>
                  </div>
                  <Button variant="danger" size="small" onClick={() => handleDeleteToken(t.id)}>
                    <TrashIcon size={14} />
                  </Button>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* ── Sessions ──────────────────────────────────────────── */}
        <div style={{ marginBottom: '32px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
            <DeviceMobileIcon size={20} />
            <h3 style={{ fontSize: '16px', fontWeight: 600, margin: 0 }}>Active Sessions</h3>
          </div>

          {sessionsLoading ? (
            <Spinner size="small" />
          ) : sessionsError ? (
            <Flash variant="danger">{sessionsError}</Flash>
          ) : sessions.length === 0 ? (
            <p style={{ color: 'var(--fg-muted)', fontSize: '14px' }}>No active sessions.</p>
          ) : (
            <div className="forge-card">
              {sessions.map((s, i) => (
                <div
                  key={s.id}
                  style={{
                    padding: '12px 16px',
                    display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                    borderBottom: i < sessions.length - 1 ? '1px solid var(--border-muted)' : undefined,
                  }}
                >
                  <div>
                    <div style={{ fontWeight: 600, fontSize: '14px' }}>
                      {s.user_agent || 'Unknown client'}
                    </div>
                    <div style={{ fontSize: '12px', color: 'var(--fg-muted)', display: 'flex', gap: '12px', marginTop: '2px' }}>
                      {s.ip && <span>{s.ip}</span>}
                      <span>Created {formatDate(s.created_at)}</span>
                      <span>Last used {timeAgo(s.last_used_at)}</span>
                      <span>Expires {formatDate(s.expires_at)}</span>
                    </div>
                  </div>
                  <Button variant="danger" size="small" onClick={() => handleDeleteSession(s.id)}>
                    Revoke
                  </Button>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
