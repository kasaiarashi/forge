import { useState, type FormEvent } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  TextInput,
  Button,
  FormControl,
  Flash,
} from '@primer/react';
import { useAuth } from '../context/AuthContext';

export default function Login() {
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const navigate = useNavigate();
  const { login } = useAuth();

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setError('');
    setLoading(true);
    try {
      const ok = await login(username, password);
      if (ok) {
        navigate('/');
      } else {
        setError('Incorrect username or password.');
      }
    } catch {
      setError('Incorrect username or password.');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="login-wrapper" style={{ marginTop: '-24px' }}>
      {/* Logo */}
      <div style={{ marginBottom: '24px' }}>
        <svg width="48" height="48" viewBox="0 0 24 24" fill="#24292f">
          <path d="M12 2L4 6v2h16V6L12 2zm-1 8H5v8l7 4 7-4v-8h-6v6l-1 .5-1-.5v-6z" />
        </svg>
      </div>

      <h1 style={{ fontSize: '24px', fontWeight: 300, color: 'var(--fg-default)', marginBottom: '16px', marginTop: 0 }}>
        Sign in to Forge
      </h1>

      {error && (
        <div style={{ width: 308, marginBottom: '16px' }}>
          <Flash variant="danger">{error}</Flash>
        </div>
      )}

      <form
        onSubmit={handleSubmit}
        style={{
          width: 308,
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
          />
        </FormControl>

        <div style={{ marginTop: '16px' }}>
          <FormControl>
            <FormControl.Label>Password</FormControl.Label>
            <TextInput
              type="password"
              block
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
            />
          </FormControl>
        </div>

        <div style={{ marginTop: '16px' }}>
          <Button
            type="submit"
            variant="primary"
            block
            disabled={loading || !username || !password}
          >
            {loading ? 'Signing in...' : 'Sign in'}
          </Button>
        </div>
      </form>

      <div
        style={{
          width: 308,
          marginTop: '16px',
          padding: '16px',
          border: '1px solid var(--border-default)',
          borderRadius: '6px',
          textAlign: 'center',
          fontSize: '12px',
          background: 'var(--bg-default)',
        }}
      >
        Forge VCS server login. Contact your admin for credentials.
      </div>
    </div>
  );
}
