import { createContext, useContext, useState, useEffect, type ReactNode } from 'react';
import type { User } from '../api';
import api from '../api';

interface AuthContextType {
  user: User | null;
  /**
   * Has the server been initialized (at least one user exists)?
   * `null` while the initial probe is in flight, then `true`/`false`.
   * Drives the `/setup` wizard gate.
   */
  initialized: boolean | null;
  loading: boolean;
  login: (username: string, password: string) => Promise<boolean>;
  logout: () => Promise<void>;
  refresh: () => Promise<void>;
  /**
   * First-admin setup. Calls /api/auth/bootstrap and then immediately
   * logs in with the same credentials so the user lands authenticated
   * after the wizard finishes.
   */
  bootstrap: (input: {
    username: string;
    email: string;
    display_name: string;
    password: string;
  }) => Promise<void>;
}

const AuthContext = createContext<AuthContextType>(null!);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<User | null>(null);
  const [initialized, setInitialized] = useState<boolean | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = async () => {
    // Probe both the server-init flag AND the current session in parallel.
    // The init flag drives the /setup gate; the user drives the /login gate.
    const [init, u] = await Promise.all([api.isInitialized(), api.me()]);
    setInitialized(init);
    setUser(u);
    setLoading(false);
  };

  useEffect(() => {
    refresh();
  }, []);

  const login = async (username: string, password: string) => {
    const res = await api.login(username, password);
    if (res.ok) {
      await refresh();
      return true;
    }
    return false;
  };

  const logout = async () => {
    await api.logout();
    setUser(null);
  };

  const bootstrap = async (input: {
    username: string;
    email: string;
    display_name: string;
    password: string;
  }) => {
    await api.bootstrapAdmin(input);
    // Mark the server initialized immediately so the gate flips before
    // the next render — the actual /api/auth/initialized refetch happens
    // inside login → refresh.
    setInitialized(true);
    const ok = await login(input.username, input.password);
    if (!ok) {
      throw new Error('Bootstrap succeeded but auto-login failed');
    }
  };

  return (
    <AuthContext.Provider
      value={{ user, initialized, loading, login, logout, refresh, bootstrap }}
    >
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth() {
  return useContext(AuthContext);
}
