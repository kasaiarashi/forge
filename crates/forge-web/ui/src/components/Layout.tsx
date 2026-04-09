import { type ReactNode } from 'react';
import { Link, useNavigate, useLocation } from 'react-router-dom';
import {
  Header,
  ActionMenu,
  ActionList,
  Avatar,
} from '@primer/react';
import {
  SignInIcon,
  SunIcon,
  MoonIcon,
  SearchIcon,
  RepoIcon,
  PlusIcon,
  BellIcon,
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
      <Header style={{ background: 'var(--header-bg)', padding: '16px', borderBottom: 'none' }}>
        <Header.Item>
          <Header.Link as={Link} to="/" style={{ color: 'var(--header-logo)' }}>
            <RepoIcon size={32} />
          </Header.Link>
        </Header.Item>

        <Header.Item>
          <div style={{ display: 'flex', alignItems: 'center', background: 'var(--header-search-bg)', border: '1px solid var(--header-search-border)', borderRadius: '6px', padding: '4px 8px', width: '272px' }}>
            <span style={{ color: 'var(--header-fg)', display: 'flex' }}><SearchIcon size={16} /></span>
            <input 
              type="text" 
              placeholder="Search or jump to..." 
              style={{ background: 'transparent', border: 'none', color: 'var(--header-fg)', outline: 'none', marginLeft: '8px', flex: 1, fontSize: '14px' }}
            />
            <div style={{ border: '1px solid var(--header-search-border)', borderRadius: '4px', padding: '0 4px', fontSize: '10px', marginLeft: '8px', color: 'var(--header-fg)' }}>/</div>
          </div>
        </Header.Item>

        <Header.Item>
          <Header.Link as={Link} to="/" style={{ color: 'var(--header-fg)', fontWeight: 600 }}>Pull requests</Header.Link>
        </Header.Item>
        <Header.Item>
          <Header.Link as={Link} to="/" style={{ color: 'var(--header-fg)', fontWeight: 600 }}>Issues</Header.Link>
        </Header.Item>

        <Header.Item full />

        {user?.is_admin && (
          <Header.Item>
            <Header.Link as={Link} to="/admin" className={linkClass('/admin')} style={{ color: 'var(--header-fg)', fontWeight: 600 }}>
              Admin
            </Header.Link>
          </Header.Item>
        )}

        <Header.Item>
          <button
            onClick={() => {
              const next = colorMode === 'auto' ? 'night' : colorMode === 'night' ? 'day' : 'auto';
              setColorMode(next);
            }}
            style={{ background: 'none', border: 'none', color: 'var(--header-fg)', cursor: 'pointer', display: 'flex', alignItems: 'center', padding: '4px' }}
            title={`Theme: ${colorMode}`}
          >
            {resolvedMode === 'night' ? <MoonIcon size={16} /> : <SunIcon size={16} />}
          </button>
        </Header.Item>

        <Header.Item>
          <Header.Link href="#" style={{ color: 'var(--header-fg)' }}><BellIcon size={16} /></Header.Link>
        </Header.Item>
        <Header.Item>
          <Header.Link href="#" style={{ color: 'var(--header-fg)', display: 'flex', alignItems: 'center', gap: '4px' }}>
            <PlusIcon size={16} />
            <span style={{ fontSize: '10px' }}>▾</span>
          </Header.Link>
        </Header.Item>

        <Header.Item>
          {loading ? null : user ? (
            <ActionMenu>
              <ActionMenu.Button variant="invisible" style={{ color: 'var(--header-fg)', padding: 0, paddingLeft: '8px' }}>
                <span style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                  <Avatar
                    src={`https://github.com/identicons/${user.username}.png`}
                    size={20}
                  />
                  <span style={{ fontSize: '10px', marginLeft: '2px' }}>▾</span>
                </span>
              </ActionMenu.Button>
              <ActionMenu.Overlay>
                <ActionList>
                  <ActionList.Item onSelect={() => navigate('/')}>
                    Your repositories
                  </ActionList.Item>
                  {user?.is_admin && (
                    <ActionList.Item onSelect={() => navigate('/admin')}>
                      Server administration
                    </ActionList.Item>
                  )}
                  <ActionList.Divider />
                  <ActionList.Item variant="danger" onSelect={handleLogout}>
                    Sign out
                  </ActionList.Item>
                </ActionList>
              </ActionMenu.Overlay>
            </ActionMenu>
          ) : (
            <Header.Link as={Link} to="/login" style={{ display: 'flex', alignItems: 'center', gap: '4px', color: 'var(--header-logo)' }}>
              <SignInIcon size={16} />
              Sign in
            </Header.Link>
          )}
        </Header.Item>
      </Header>

      <main style={{ flex: 1, maxWidth: 1280, margin: '0 auto', width: '100%', padding: '24px 16px' }}>
        {children}
      </main>

      {/* `gap: 32px` + `flex-wrap: wrap` keeps the © block and the link
          row from kissing on wide screens AND lets them stack cleanly on
          narrow ones. `space-between` still expands the gap to fill the
          row when there's room. */}
      <footer className="forge-footer" style={{ borderTop: 'none', marginTop: '40px', paddingTop: '40px', paddingBottom: '40px', maxWidth: '1012px', margin: '40px auto 0 auto', display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexWrap: 'wrap', gap: '32px', color: 'var(--fg-muted)', fontSize: '12px', background: 'transparent' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '12px', marginRight: '24px' }}>
          <span style={{ color: 'var(--fg-muted)', display: 'flex' }}><RepoIcon size={24} /></span>
          {/* Non-breaking space after the © glyph so the year doesn't
              wrap onto a new line on narrow viewports. */}
          <span>©&nbsp;2026 Forge VCS</span>
        </div>
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: '16px' }}>
          <FooterLink to="/terms">Terms</FooterLink>
          <FooterLink to="/privacy">Privacy</FooterLink>
          <FooterLink to="/security">Security</FooterLink>
          <FooterLink to="/status">Status</FooterLink>
          <FooterLink to="/docs">Docs</FooterLink>
          <FooterLink to="/contact">Contact</FooterLink>
        </div>
      </footer>
    </div>
  );
}

/**
 * Footer link styled like the rest of the footer row, but routed via
 * react-router so navigation stays inside the SPA (no full-page reload).
 */
function FooterLink({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <Link
      to={to}
      style={{ color: 'var(--fg-accent)', textDecoration: 'none' }}
      onMouseOver={(e) => (e.currentTarget.style.textDecoration = 'underline')}
      onMouseOut={(e) => (e.currentTarget.style.textDecoration = 'none')}
    >
      {children}
    </Link>
  );
}
