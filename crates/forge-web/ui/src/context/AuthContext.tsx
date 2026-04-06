import { createContext, useContext, useState, useEffect, type ReactNode } from 'react';
import type { User } from '../api';
import api from '../api';

interface AuthContextType {
  user: User | null;
  loading: boolean;
  login: (username: string, password: string) => Promise<boolean>;
  logout: () => Promise<void>;
  refresh: () => Promise<void>;
}

const AuthContext = createContext<AuthContextType>(null!);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = async () => {
    const u = await api.me();
    setUser(u);
    setLoading(false);
  };

  useEffect(() => { refresh(); }, []);

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

  return (
    <AuthContext.Provider value={{ user, loading, login, logout, refresh }}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth() {
  return useContext(AuthContext);
}
