import { Link } from 'react-router-dom';
import { UnderlineNav, Label } from '@primer/react';
import {
  RepoIcon,
  CodeIcon,
  GitCommitIcon,
  PlayIcon,
  LockIcon,
  TagIcon,
  GearIcon,
} from '@primer/octicons-react';

type Tab = 'code' | 'commits' | 'actions' | 'locks' | 'releases' | 'settings';

interface RepoHeaderProps {
  repo: string;
  currentTab: Tab;
  activeBranch?: string;
}

export default function RepoHeader({ repo, currentTab, activeBranch = 'main' }: RepoHeaderProps) {
  const encRepo = encodeURIComponent(repo);
  const encBranch = encodeURIComponent(activeBranch);

  return (
    <>
      {/* Repo name header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '6px', marginBottom: '16px' }}>
        <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}>
          <RepoIcon size={16} />
        </span>
        <Link to={`/${encRepo}`} style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-accent)', textDecoration: 'none' }}>
          {repo}
        </Link>
        <Label size="small" variant="secondary" style={{ marginLeft: '4px' }}>
          Private
        </Label>
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
          to={`/${encRepo}/commits/${encBranch}`}
          aria-current={currentTab === 'commits' ? 'page' : undefined}
          icon={GitCommitIcon}
        >
          Commits
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
        <UnderlineNav.Item
          as={Link}
          to={`/${encRepo}/settings`}
          aria-current={currentTab === 'settings' ? 'page' : undefined}
          icon={GearIcon}
        >
          Settings
        </UnderlineNav.Item>
      </UnderlineNav>
    </>
  );
}
