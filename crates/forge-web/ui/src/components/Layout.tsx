import { type ReactNode } from 'react';
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
  SunIcon,
  MoonIcon,
} from '@primer/octicons-react';
import { useAuth } from '../context/AuthContext';
import { useTheme } from '../context/ThemeContext';

interface LayoutProps {
  children: ReactNode;
}

export default function Layout({ children }: LayoutProps) {
  const { user, loading, logout } = useAuth();
  const { colorMode, setColorMode, resolvedMode } = useTheme();
  const navigate = useNavigate();
  const location = useLocation();

  const handleLogout = async () => {
    await logout();
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
          <button
            onClick={() => {
              const next = colorMode === 'auto' ? 'night' : colorMode === 'night' ? 'day' : 'auto';
              setColorMode(next);
            }}
            style={{ background: 'none', border: 'none', color: 'white', cursor: 'pointer', display: 'flex', alignItems: 'center', padding: '4px' }}
            title={`Theme: ${colorMode}`}
          >
            {resolvedMode === 'night' ? <MoonIcon size={16} /> : <SunIcon size={16} />}
          </button>
        </Header.Item>

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
                  <ActionList.Item onSelect={() => navigate('/')}>
                    <ActionList.LeadingVisual>
                      <RepoIcon size={16} />
                    </ActionList.LeadingVisual>
                    Your repositories
                  </ActionList.Item>
                  {user?.is_admin && (
                    <ActionList.Item onSelect={() => navigate('/admin')}>
                      <ActionList.LeadingVisual>
                        <GearIcon size={16} />
                      </ActionList.LeadingVisual>
                      Server administration
                    </ActionList.Item>
                  )}
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
