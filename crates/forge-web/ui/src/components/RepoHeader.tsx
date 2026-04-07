import { Link } from 'react-router-dom';
import { UnderlineNav, Label, Button } from '@primer/react';
import { useState } from 'react';
import {
  RepoIcon,
  CodeIcon,
  GitCommitIcon,
  PlayIcon,
  LockIcon,
  TagIcon,
  GearIcon,
  IssueOpenedIcon,
  GitPullRequestIcon,
  ProjectIcon,
  EyeIcon,
  RepoForkedIcon,
  StarIcon,
  ShieldIcon,
  GraphIcon,
  StarFillIcon,
} from '@primer/octicons-react';
import { useAuth } from '../context/AuthContext';

type Tab = 'code' | 'commits' | 'actions' | 'locks' | 'releases' | 'settings' | 'issues' | 'pulls' | 'projects' | 'security' | 'insights';

interface RepoHeaderProps {
  repo: string;
  currentTab: Tab;
  activeBranch?: string;
}

export default function RepoHeader({ repo, currentTab, activeBranch }: RepoHeaderProps) {
  const encRepo = encodeURIComponent(repo);

  // Persist branch per repo so navigating between tabs preserves it
  const storageKey = `forge-branch-${repo}`;
  if (activeBranch && activeBranch !== 'main') {
    localStorage.setItem(storageKey, activeBranch);
  }
  const resolvedBranch = activeBranch || localStorage.getItem(storageKey) || 'main';
  const encBranch = encodeURIComponent(resolvedBranch);
  const { user } = useAuth();
  
  const owner = user?.username || 'user';
  
  // Interactive mock states
  const [isWatching, setIsWatching] = useState(false);
  const [isStarred, setIsStarred] = useState(false);
  const [starCount, setStarCount] = useState(Math.floor(Math.random() * 100));
  const [forkCount, setForkCount] = useState(Math.floor(Math.random() * 50));
  const watchCount = isWatching ? 1 : 0;

  const toggleWatch = () => setIsWatching(!isWatching);
  const toggleStar = () => {
    if (isStarred) {
      setIsStarred(false);
      setStarCount(s => s - 1);
    } else {
      setIsStarred(true);
      setStarCount(s => s + 1);
    }
  };
  const handleFork = () => setForkCount(f => f + 1);

  return (
    <div style={{ background: 'var(--bg-default)', borderBottom: '1px solid var(--border-default)', paddingTop: '16px', marginBottom: '24px', margin: '-24px -16px 24px -16px', paddingLeft: '16px', paddingRight: '16px' }}>
      <div style={{ maxWidth: '1280px', margin: '0 auto' }}>
        <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', marginBottom: '16px' }}>
          
          {/* Breadcrumb Title */}
          <div style={{ display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: '8px' }}>
            <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}>
              <RepoIcon size={16} />
            </span>
            <div style={{ fontSize: '20px', display: 'flex', alignItems: 'center' }}>
              <Link to={`/${encodeURIComponent(owner)}`} style={{ color: 'var(--fg-accent)', textDecoration: 'none' }} onMouseOver={(e) => (e.currentTarget.style.textDecoration = 'underline')} onMouseOut={(e) => (e.currentTarget.style.textDecoration = 'none')}>
                {owner}
              </Link>
              <span style={{ margin: '0 4px', color: 'var(--fg-muted)' }}>/</span>
              <Link to={`/${encRepo}`} style={{ fontWeight: 600, color: 'var(--fg-accent)', textDecoration: 'none' }} onMouseOver={(e) => (e.currentTarget.style.textDecoration = 'underline')} onMouseOut={(e) => (e.currentTarget.style.textDecoration = 'none')}>
                {repo}
              </Link>
            </div>
            <Label size="small" variant="secondary" style={{ marginLeft: '4px', alignSelf: 'center' }}>
              Public
            </Label>
          </div>

          {/* Action Buttons */}
          <div style={{ display: 'flex', gap: '8px' }}>
            <div style={{ display: 'flex' }}>
              <Button size="small" leadingVisual={EyeIcon} style={{ borderTopRightRadius: 0, borderBottomRightRadius: 0, paddingRight: '8px' }} onClick={toggleWatch}>
                {isWatching ? 'Unwatch' : 'Watch'} <span style={{ marginLeft: '4px', fontSize: '10px' }}>▾</span>
              </Button>
              <Button size="small" style={{ borderTopLeftRadius: 0, borderBottomLeftRadius: 0, borderLeft: 0 }}>{watchCount}</Button>
            </div>
            <div style={{ display: 'flex' }}>
              <Button size="small" leadingVisual={RepoForkedIcon} style={{ borderTopRightRadius: 0, borderBottomRightRadius: 0 }} onClick={handleFork}>
                Fork <span style={{ marginLeft: '4px', fontSize: '10px' }}>▾</span>
              </Button>
              <Button size="small" style={{ borderTopLeftRadius: 0, borderBottomLeftRadius: 0, borderLeft: 0 }}>{forkCount}</Button>
            </div>
            <div style={{ display: 'flex' }}>
              <Button size="small" leadingVisual={isStarred ? StarFillIcon : StarIcon} style={{ borderTopRightRadius: 0, borderBottomRightRadius: 0 }} onClick={toggleStar}>
                {isStarred ? 'Unstar' : 'Star'}
              </Button>
              <Button size="small" style={{ borderTopLeftRadius: 0, borderBottomLeftRadius: 0, borderLeft: 0 }}>{starCount}</Button>
            </div>
          </div>
        </div>

        {/* Repository tabs */}
        <UnderlineNav aria-label="Repository">
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/tree/${encBranch}`}
            aria-current={currentTab === 'code' ? 'page' : undefined}
            icon={CodeIcon}
          >
            Code
          </UnderlineNav.Item>
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/issues`}
            aria-current={currentTab === 'issues' ? 'page' : undefined}
            icon={IssueOpenedIcon}
          >
            Issues
          </UnderlineNav.Item>
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/pulls`}
            aria-current={currentTab === 'pulls' ? 'page' : undefined}
            icon={GitPullRequestIcon}
          >
            Pull requests
          </UnderlineNav.Item>
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/actions`}
            aria-current={currentTab === 'actions' ? 'page' : undefined}
            icon={PlayIcon}
          >
            Actions
          </UnderlineNav.Item>
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/projects`}
            aria-current={currentTab === 'projects' ? 'page' : undefined}
            icon={ProjectIcon}
          >
            Projects
          </UnderlineNav.Item>
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/security`}
            aria-current={currentTab === 'security' ? 'page' : undefined}
            icon={ShieldIcon}
          >
            Security
          </UnderlineNav.Item>
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/insights`}
            aria-current={currentTab === 'insights' ? 'page' : undefined}
            icon={GraphIcon}
          >
            Insights
          </UnderlineNav.Item>
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/settings`}
            aria-current={currentTab === 'settings' ? 'page' : undefined}
            icon={GearIcon}
          >
            Settings
          </UnderlineNav.Item>
          
          {/* Include original Forge-specific tabs */}
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/commits/${encBranch}`}
            aria-current={currentTab === 'commits' ? 'page' : undefined}
            icon={GitCommitIcon}
          >
            Commits
          </UnderlineNav.Item>
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/locks`}
            aria-current={currentTab === 'locks' ? 'page' : undefined}
            icon={LockIcon}
          >
            Locks
          </UnderlineNav.Item>
          <UnderlineNav.Item
            as={Link}
            to={`/${encRepo}/releases`}
            aria-current={currentTab === 'releases' ? 'page' : undefined}
            icon={TagIcon}
          >
            Releases
          </UnderlineNav.Item>
        </UnderlineNav>
      </div>
    </div>
  );
}
