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
  IssueOpenedIcon,
  GitPullRequestIcon,
} from '@primer/octicons-react';
import { repoPath, splitRepo } from '../api';

export type Tab = 'code' | 'commits' | 'actions' | 'locks' | 'releases' | 'settings' | 'issues' | 'pulls';

interface RepoHeaderProps {
  /**
   * Full `<owner>/<name>` identifier — e.g. "alice/forge". The breadcrumb
   * splits it for display so we don't accidentally render "alice / alice/forge".
   */
  repo: string;
  currentTab: Tab;
  activeBranch?: string;
  visibility?: string; // "public" | "private"
}

export default function RepoHeader({ repo, currentTab, activeBranch, visibility }: RepoHeaderProps) {
  // `repo` is the full `owner/name` path. Build navigation links via
  // repoPath() so each segment is encoded but the / between them stays
  // literal (so React Router's :owner/:repo can match). Split into halves
  // for the visible breadcrumb so we don't double up the owner.
  const encRepo = repoPath(repo);
  const [ownerSeg, repoNameSeg] = splitRepo(repo);

  // Persist branch per repo so navigating between tabs preserves it
  const storageKey = `forge-branch-${repo}`;
  if (activeBranch && activeBranch !== 'main') {
    localStorage.setItem(storageKey, activeBranch);
  }
  const resolvedBranch = activeBranch || localStorage.getItem(storageKey) || 'main';
  const encBranch = encodeURIComponent(resolvedBranch);

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
              <Link to={`/${encodeURIComponent(ownerSeg)}`} style={{ color: 'var(--fg-accent)', textDecoration: 'none' }} onMouseOver={(e) => (e.currentTarget.style.textDecoration = 'underline')} onMouseOut={(e) => (e.currentTarget.style.textDecoration = 'none')}>
                {ownerSeg}
              </Link>
              <span style={{ margin: '0 4px', color: 'var(--fg-muted)' }}>/</span>
              <Link to={`/${encRepo}`} style={{ fontWeight: 600, color: 'var(--fg-accent)', textDecoration: 'none' }} onMouseOver={(e) => (e.currentTarget.style.textDecoration = 'underline')} onMouseOut={(e) => (e.currentTarget.style.textDecoration = 'none')}>
                {repoNameSeg}
              </Link>
            </div>
            <Label size="small" variant="secondary" style={{ marginLeft: '4px', alignSelf: 'center' }}>
              {visibility === 'public' ? 'Public' : 'Private'}
            </Label>
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
