import { useState, type FormEvent } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  TextInput,
  Button,
  FormControl,
  Flash,
} from '@primer/react';
import { useAuth } from '../context/AuthContext';

/**
 * First-run setup wizard. Renders only when `/api/auth/initialized` returns
 * false; once a user exists the route guard in App.tsx redirects callers
 * to `/login` instead. The form posts to `/api/auth/bootstrap` (which is
 * itself one-shot — forge-server rejects subsequent calls) and then auto-
 * logs in via the same credentials so the operator lands on the dashboard
 * already authenticated.
 */
export default function Setup() {
  const navigate = useNavigate();
  const { bootstrap } = useAuth();

  const [username, setUsername] = useState('');
  const [email, setEmail] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [bootstrapToken, setBootstrapToken] = useState('');
  const [error, setError] = useState('');
  const [submitting, setSubmitting] = useState(false);

  const usernameOk = /^[a-zA-Z0-9._-]{1,64}$/.test(username);
  const emailOk = /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
  const passwordOk = password.length >= 8;
  const matches = password === confirm;
  const tokenOk = bootstrapToken.trim().length >= 32;
  const canSubmit =
    usernameOk && emailOk && passwordOk && matches && tokenOk && !submitting;

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setError('');
    if (!canSubmit) return;
    setSubmitting(true);
    try {
      await bootstrap({
        username: username.trim(),
        email: email.trim(),
        display_name: displayName.trim() || username.trim(),
        password,
        bootstrap_token: bootstrapToken.trim(),
      });
      // Auto-login fired inside bootstrap(); land on the dashboard.
      navigate('/');
    } catch (e: any) {
      setError(e?.message?.replace(/^\d+:\s*/, '') || 'Failed to create admin');
      setSubmitting(false);
    }
  };

  return (
    <div className="login-wrapper" style={{ marginTop: '-24px' }}>
      <div style={{ marginBottom: '24px' }}>
        <svg width="48" height="48" viewBox="0 0 24 24" fill="#24292f">
          <path d="M12 2L4 6v2h16V6L12 2zm-1 8H5v8l7 4 7-4v-8h-6v6l-1 .5-1-.5v-6z" />
        </svg>
      </div>

      <h1
        style={{
          fontSize: '24px',
          fontWeight: 300,
          color: 'var(--fg-default)',
          marginBottom: '8px',
          marginTop: 0,
        }}
      >
        Welcome to Forge VCS
      </h1>
      <p
        style={{
          fontSize: '14px',
          color: 'var(--fg-muted)',
          maxWidth: 400,
          textAlign: 'center',
          margin: '0 0 24px 0',
        }}
      >
        This forge server has no users yet. Create the first administrator
        account below — you'll be able to add more users from the admin panel
        afterwards.
      </p>

      {error && (
        <div style={{ width: 360, marginBottom: '16px' }}>
          <Flash variant="danger">{error}</Flash>
        </div>
      )}

      <form
        onSubmit={handleSubmit}
        style={{
          width: 360,
          padding: '16px',
          background: 'var(--bg-default)',
          border: '1px solid var(--border-default)',
          borderRadius: '6px',
        }}
      >
        <FormControl>
          <FormControl.Label>Username</FormControl.Label>
          <TextInput
            block
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoFocus
            autoComplete="username"
            placeholder="alice"
            aria-invalid={username.length > 0 && !usernameOk}
          />
          {username.length > 0 && !usernameOk && (
            <FormControl.Validation variant="error">
              1–64 characters: letters, digits, '.', '_', '-'
            </FormControl.Validation>
          )}
        </FormControl>

        <div style={{ marginTop: '16px' }}>
          <FormControl>
            <FormControl.Label>Email</FormControl.Label>
            <TextInput
              type="email"
              block
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              autoComplete="email"
              placeholder="you@example.com"
              aria-invalid={email.length > 0 && !emailOk}
            />
            {email.length > 0 && !emailOk && (
              <FormControl.Validation variant="error">
                Enter a valid email address
              </FormControl.Validation>
            )}
          </FormControl>
        </div>

        <div style={{ marginTop: '16px' }}>
          <FormControl>
            <FormControl.Label>Display name (optional)</FormControl.Label>
            <TextInput
              block
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              autoComplete="name"
              placeholder="Alice Smith"
            />
            <FormControl.Caption>
              How you'll appear in commits and the UI. Defaults to your username.
            </FormControl.Caption>
          </FormControl>
        </div>

        <div style={{ marginTop: '16px' }}>
          <FormControl>
            <FormControl.Label>Password</FormControl.Label>
            <TextInput
              type="password"
              block
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="new-password"
              aria-invalid={password.length > 0 && !passwordOk}
            />
            {password.length > 0 && !passwordOk && (
              <FormControl.Validation variant="error">
                Password must be at least 8 characters
              </FormControl.Validation>
            )}
          </FormControl>
        </div>

        <div style={{ marginTop: '16px' }}>
          <FormControl>
            <FormControl.Label>Confirm password</FormControl.Label>
            <TextInput
              type="password"
              block
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              autoComplete="new-password"
              aria-invalid={confirm.length > 0 && !matches}
            />
            {confirm.length > 0 && !matches && (
              <FormControl.Validation variant="error">
                Passwords don't match
              </FormControl.Validation>
            )}
          </FormControl>
        </div>

        <div style={{ marginTop: '16px' }}>
          <FormControl>
            <FormControl.Label>Bootstrap token</FormControl.Label>
            <TextInput
              block
              value={bootstrapToken}
              onChange={(e) => setBootstrapToken(e.target.value)}
              autoComplete="off"
              placeholder="hex token from the forge-server logs"
              aria-invalid={bootstrapToken.length > 0 && !tokenOk}
              monospace
            />
            <FormControl.Caption>
              Printed by <code>forge-server</code> on first start, also saved
              to <code>&lt;base_path&gt;/.bootstrap_token</code>. Consumed after
              the first admin is created.
            </FormControl.Caption>
            {bootstrapToken.length > 0 && !tokenOk && (
              <FormControl.Validation variant="error">
                Token looks too short — paste the full hex value
              </FormControl.Validation>
            )}
          </FormControl>
        </div>

        <div style={{ marginTop: '24px' }}>
          <Button
            type="submit"
            variant="primary"
            block
            disabled={!canSubmit}
          >
            {submitting ? 'Creating administrator…' : 'Create administrator'}
          </Button>
        </div>
      </form>

      <div
        style={{
          width: 360,
          marginTop: '16px',
          padding: '12px',
          border: '1px solid var(--border-default)',
          borderRadius: '6px',
          fontSize: '12px',
          background: 'var(--bg-default)',
          color: 'var(--fg-muted)',
        }}
      >
        Prefer the command line? Run:
        <pre
          style={{
            margin: '6px 0 0 0',
            padding: '8px',
            background: 'var(--bg-canvas-inset)',
            borderRadius: '4px',
            fontSize: '11px',
            overflow: 'auto',
          }}
        >
forge-server user add --admin &lt;username&gt;
        </pre>
      </div>
    </div>
  );
}
