import { Link } from 'react-router-dom';
import { UnderlineNav, Label, Button } from '@primer/react';
import {
  CodeIcon,
  IssueOpenedIcon,
  GitPullRequestIcon,
  PlayIcon,
  GearIcon,
  PinIcon,
} from '@primer/octicons-react';
import { useEffect, useState } from 'react';
import type { RepoInfo } from '../api';
import api, { repoPath } from '../api';

export type Tab = 'code' | 'commits' | 'actions' | 'locks' | 'releases' | 'settings' | 'issues' | 'pulls' | 'projects' | 'wiki' | 'security' | 'insights';

export default function RepoHeader({ repo, currentTab, visibility: externalVisibility }: { repo: string, currentTab: string, activeBranch?: string, visibility?: string }) {
  const encRepo = repoPath(repo);

  const [repoInfo, setRepoInfo] = useState<RepoInfo | null>(null);

  useEffect(() => {
    api.listRepos().then(repos => {
      const match = repos.find(r => r.name === repo);
      if (match) setRepoInfo(match);
    }).catch(() => {});
  }, [repo]);

  const owner = repo.split('/')[0];
  const name = repoInfo?.name || repo.split('/')[1] || repo;
  const visibility = externalVisibility || repoInfo?.visibility || 'public';

  return (
    <>
      <div style={{ background: 'var(--bg-default)', borderBottom: '1px solid var(--border-default)', padding: '0 var(--space-6)' }}>
        <UnderlineNav aria-label="Repository">
          <UnderlineNav.Item as={Link} to={`/${encRepo}`} aria-current={currentTab === 'code' ? 'page' : undefined} icon={CodeIcon}>
            Code
          </UnderlineNav.Item>
          <UnderlineNav.Item as={Link} to={`/${encRepo}/issues`} aria-current={currentTab === 'issues' ? 'page' : undefined} icon={IssueOpenedIcon}>
            Issues
          </UnderlineNav.Item>
          <UnderlineNav.Item as={Link} to={`/${encRepo}/pulls`} aria-current={currentTab === 'pulls' ? 'page' : undefined} icon={GitPullRequestIcon}>
            Pull requests
          </UnderlineNav.Item>
          <UnderlineNav.Item as={Link} to={`/${encRepo}/actions`} aria-current={currentTab === 'actions' ? 'page' : undefined} icon={PlayIcon}>
            Actions
          </UnderlineNav.Item>

          <UnderlineNav.Item as={Link} to={`/${encRepo}/settings`} aria-current={currentTab === 'settings' ? 'page' : undefined} icon={GearIcon}>
            Settings
          </UnderlineNav.Item>
        </UnderlineNav>
      </div>

      <div style={{ maxWidth: '1280px', margin: '0 auto', padding: 'var(--space-6) var(--space-6) 0 var(--space-6)' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 'var(--space-4)' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
            <div className="avatar-circle" style={{ width: 24, height: 24, fontSize: 12, marginRight: 'var(--space-1)' }}>
              {owner.charAt(0).toUpperCase()}
            </div>
            <Link to={`/${encRepo}`} style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-default)', textDecoration: 'none' }}>
              {name}
            </Link>
            <Label variant="secondary" style={{ borderRadius: '2em', marginLeft: 'var(--space-1)' }}>
              {visibility === 'private' ? 'Private' : 'Public'}
            </Label>
          </div>

          <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-2)' }}>
            <Button size="small" leadingVisual={PinIcon}>Unpin</Button>
          </div>
        </div>
      </div>
    </>
  );
}
