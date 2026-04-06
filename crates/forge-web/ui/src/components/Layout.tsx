import { type ReactNode, useEffect, useState, useCallback } from 'react';
import { Link, useNavigate, useLocation } from 'react-router-dom';
import {
  Header,
  ActionMenu,
  ActionList,
  Avatar,
} from '@primer/react';
import {
  RepoIcon,
  GearIcon,
  SignOutIcon,
  SignInIcon,
  PersonIcon,
} from '@primer/octicons-react';
import type { User } from '../api';
import api from '../api';

interface LayoutProps {
  children: ReactNode;
}

export default function Layout({ children }: LayoutProps) {
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(true);
  const navigate = useNavigate();
  const location = useLocation();

  const fetchUser = useCallback(() => {
    api.me().then((u) => {
      setUser(u);
      setLoading(false);
    });
  }, []);

  useEffect(() => {
    fetchUser();
  }, [fetchUser]);

  const handleLogout = async () => {
    await api.logout();
    setUser(null);
    navigate('/login');
  };

  const isActive = (path: string) =>
    location.pathname === path || location.pathname.startsWith(path + '/');

  const linkClass = (path: string) =>
    isActive(path) ? 'active-link' : '';

  return (
    <div style={{ display: 'flex', flexDirection: 'column', minHeight: '100vh' }}>
      <Header>
        <Header.Item>
          <Header.Link as={Link} to="/" style={{ fontWeight: 'bold', fontSize: '16px', display: 'flex', alignItems: 'center', gap: '8px' }}>
            <svg width="32" height="32" viewBox="0 0 24 24" fill="white">
              <path d="M12 2L4 6v2h16V6L12 2zm-1 8H5v8l7 4 7-4v-8h-6v6l-1 .5-1-.5v-6z" />
            </svg>
            Forge
          </Header.Link>
        </Header.Item>

        <Header.Item>
          <Header.Link as={Link} to="/" className={linkClass('/')} style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
            <RepoIcon size={16} />
            Repositories
          </Header.Link>
        </Header.Item>

        {user?.is_admin && (
          <Header.Item>
            <Header.Link as={Link} to="/admin" className={linkClass('/admin')} style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
              <GearIcon size={16} />
              Admin
            </Header.Link>
          </Header.Item>
        )}

        <Header.Item full />

        <Header.Item>
          {loading ? null : user ? (
            <ActionMenu>
              <ActionMenu.Button>
                <span style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <Avatar
                    src={`https://github.com/identicons/${user.username}.png`}
                    size={20}
                  />
                  {user.username}
                </span>
              </ActionMenu.Button>
              <ActionMenu.Overlay>
                <ActionList>
                  <ActionList.Item onSelect={() => navigate('/admin')}>
                    <ActionList.LeadingVisual>
                      <PersonIcon size={16} />
                    </ActionList.LeadingVisual>
                    Profile
                  </ActionList.Item>
                  <ActionList.Divider />
                  <ActionList.Item variant="danger" onSelect={handleLogout}>
                    <ActionList.LeadingVisual>
                      <SignOutIcon size={16} />
                    </ActionList.LeadingVisual>
                    Sign out
                  </ActionList.Item>
                </ActionList>
              </ActionMenu.Overlay>
            </ActionMenu>
          ) : (
            <Header.Link as={Link} to="/login" style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
              <SignInIcon size={16} />
              Sign in
            </Header.Link>
          )}
        </Header.Item>
      </Header>

      <main style={{ flex: 1, maxWidth: 1280, margin: '0 auto', width: '100%', padding: '24px 16px' }}>
        {children}
      </main>

      <footer className="forge-footer">
        Forge VCS &mdash; Binary-first version control for Unreal Engine
      </footer>
    </div>
  );
}
