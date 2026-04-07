import { useEffect, useState } from 'react';
import { useParams, Link, useNavigate } from 'react-router-dom';
import {
  Breadcrumbs,
  ActionMenu,
  ActionList,
  Spinner,
  Flash,
  UnderlineNav,
  TextInput,
  Button,
  Label,
} from '@primer/react';
import {
  FileDirectoryFillIcon,
  FileIcon,
  GitBranchIcon,
  CodeIcon,
  GitCommitIcon,
  LockIcon,
  GearIcon,
  RepoIcon,
  CopyIcon,
  HistoryIcon,
  TagIcon,
  SearchIcon,
} from '@primer/octicons-react';
import type { TreeEntry, Branch, CommitSummary, RepoInfo } from '../api';
import api, { copyToClipboard } from '../api';

function timeAgo(epoch: number): string {
  if (!epoch) return '';
  const seconds = Math.floor((Date.now() - epoch * 1000) / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes !== 1 ? 's' : ''} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours !== 1 ? 's' : ''} ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days} day${days !== 1 ? 's' : ''} ago`;
  const weeks = Math.floor(days / 7);
  if (weeks < 5) return `${weeks} week${weeks !== 1 ? 's' : ''} ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months} month${months !== 1 ? 's' : ''} ago`;
  return `${Math.floor(days / 365)} year${Math.floor(days / 365) !== 1 ? 's' : ''} ago`;
}

export default function RepoTree() {
  const { repo = '', branch, '*': pathStr = '' } = useParams();
  const path = pathStr || '';
  const navigate = useNavigate();

  const [entries, setEntries] = useState<TreeEntry[]>([]);
  const [branches, setBranches] = useState<Branch[]>([]);
  const [activeBranch, setActiveBranch] = useState(branch || '');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [latestCommit, setLatestCommit] = useState<CommitSummary | null>(null);
  const [commitCount, setCommitCount] = useState(0);
  const [repoInfo, setRepoInfo] = useState<RepoInfo | null>(null);
  const [cloneCopied, setCloneCopied] = useState(false);
  const [showCloneMenu, setShowCloneMenu] = useState(false);

  const cloneUrl = `${window.location.protocol}//${window.location.hostname}:9876`;

  const copyCloneUrl = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    copyToClipboard(`forge clone ${cloneUrl}`);
    setCloneCopied(true);
    setTimeout(() => setCloneCopied(false), 2000);
  };

  useEffect(() => {
    setLoading(true);
    setError('');

    const loadData = async () => {
      try {
        const [br, repos] = await Promise.all([
          api.listBranches(repo),
          api.listRepos(),
        ]);
        setBranches(br);
        const ri = repos.find(r => r.name === repo) || null;
        setRepoInfo(ri);

        const resolvedBranch = branch || (br.find(b => b.name === 'main') || br[0])?.name || 'main';
        setActiveBranch(resolvedBranch);

        const [tree, commits] = await Promise.all([
          api.getTree(repo, resolvedBranch, path),
          api.listCommits(repo, resolvedBranch, 1, 1),
        ]);

        const sorted = [...tree.entries].sort((a, b) => {
          if (a.kind !== b.kind) return a.kind === 'directory' ? -1 : 1;
          return a.name.localeCompare(b.name);
        });
        setEntries(sorted);
        setLatestCommit(commits.commits[0] || null);
        setCommitCount(commits.total);
      } catch (e: any) {
        setError(e.message);
      } finally {
        setLoading(false);
      }
    };

    loadData();
  }, [repo, branch, path]);

  const pathParts = path ? path.split('/') : [];
  const encRepo = encodeURIComponent(repo);
  const encBranch = encodeURIComponent(activeBranch);

  const buildPath = (index: number): string => {
    const parts = pathParts.slice(0, index + 1).join('/');
    return `/${encRepo}/tree/${encBranch}/${parts}`;
  };

  const getEntryLink = (entry: TreeEntry): string => {
    const entryPath = path ? `${path}/${entry.name}` : entry.name;
    if (entry.kind === 'directory') {
      return `/${encRepo}/tree/${encBranch}/${entryPath}`;
    }
    return `/${encRepo}/blob/${encBranch}/${entryPath}`;
  };

  if (loading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
        <Spinner size="large" />
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ padding: '24px 0' }}>
        <Flash variant="danger">{error}</Flash>
      </div>
    );
  }

  return (
    <div>
      {/* Repo name header — like GitHub: owner / repo-name */}
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

      {/* Repository tabs — Code, Commits, Locks, Settings */}
      <UnderlineNav aria-label="Repository">
        <UnderlineNav.Item as={Link} to={`/${encRepo}/tree/${encBranch}`} aria-current="page" icon={CodeIcon}>
          Code
        </UnderlineNav.Item>
        <UnderlineNav.Item as={Link} to={`/${encRepo}/commits/${encBranch}`} icon={GitCommitIcon}>
          Commits
        </UnderlineNav.Item>
        <UnderlineNav.Item as={Link} to={`/${encRepo}/locks`} icon={LockIcon}>
          Locks
        </UnderlineNav.Item>
        <UnderlineNav.Item as={Link} to={`/${encRepo}/settings`} icon={GearIcon}>
          Settings
        </UnderlineNav.Item>
      </UnderlineNav>

      {/* Main content: left file browser + right sidebar */}
      <div style={{ display: 'flex', gap: '24px', marginTop: '16px' }}>
        {/* Left: file browser */}
        <div style={{ flex: 1, minWidth: 0 }}>

          {/* Top toolbar: branch selector, branches count, search, Code button */}
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px', flexWrap: 'wrap' }}>
            {/* Branch selector */}
            <ActionMenu>
              <ActionMenu.Button leadingVisual={GitBranchIcon} size="small">
                {activeBranch}
              </ActionMenu.Button>
              <ActionMenu.Overlay width="medium">
                <ActionList>
                  <ActionList.GroupHeading>Switch branches</ActionList.GroupHeading>
                  {branches.map((b) => (
                    <ActionList.Item
                      key={b.name}
                      selected={b.name === activeBranch}
                      onSelect={() => navigate(`/${encRepo}/tree/${encodeURIComponent(b.name)}`)}
                    >
                      {b.name}
                    </ActionList.Item>
                  ))}
                </ActionList>
              </ActionMenu.Overlay>
            </ActionMenu>

            {/* Branch/tag counts */}
            <Link to={`/${encRepo}/tree/${encBranch}`} style={{ display: 'flex', alignItems: 'center', gap: '4px', fontSize: '13px', color: 'var(--fg-muted)', textDecoration: 'none' }}>
              <GitBranchIcon size={14} />
              <strong style={{ color: 'var(--fg-default)' }}>{branches.length}</strong> Branch{branches.length !== 1 ? 'es' : ''}
            </Link>
            <span style={{ display: 'flex', alignItems: 'center', gap: '4px', fontSize: '13px', color: 'var(--fg-muted)' }}>
              <TagIcon size={14} />
              <strong style={{ color: 'var(--fg-default)' }}>0</strong> Tags
            </span>

            <div style={{ flex: 1 }} />

            {/* Go to file search */}
            <Button size="small" leadingVisual={SearchIcon} variant="invisible" style={{ color: 'var(--fg-muted)' }}>
              Go to file
            </Button>

            {/* Green Code button */}
            <div style={{ position: 'relative' }}>
              <Button variant="primary" size="small" leadingVisual={CodeIcon} trailingAction={() => <span>▾</span>} onClick={() => setShowCloneMenu(!showCloneMenu)}>
                Code
              </Button>
              {showCloneMenu && (
                <div style={{
                  position: 'absolute', right: 0, top: '100%', marginTop: '4px', zIndex: 100,
                  width: 360, padding: '16px', borderRadius: '6px',
                  background: 'var(--bg-default)', border: '1px solid var(--border-default)',
                  boxShadow: '0 8px 24px rgba(0,0,0,0.2)',
                }}>
                  <div style={{ fontWeight: 600, fontSize: '14px', marginBottom: '8px', color: 'var(--fg-default)' }}>Clone</div>
                  <div style={{ fontSize: '12px', color: 'var(--fg-muted)', marginBottom: '8px' }}>
                    Use Forge CLI to clone this repository.
                  </div>
                  <div style={{ display: 'flex', gap: '4px', marginBottom: '12px' }}>
                    <TextInput value={`forge clone ${cloneUrl}`} readOnly block monospace size="small" />
                    <Button size="small" onClick={copyCloneUrl}>
                      {cloneCopied ? 'Copied!' : <CopyIcon size={16} />}
                    </Button>
                  </div>
                  <div style={{ fontSize: '12px', color: 'var(--fg-muted)' }}>
                    Then: <code style={{ background: 'var(--bg-subtle)', padding: '2px 6px', borderRadius: '3px' }}>forge config repo {repo}</code>
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* Breadcrumb (only in subfolders) */}
          {pathParts.length > 0 && (
            <div style={{ marginBottom: '8px' }}>
              <Breadcrumbs>
                <Breadcrumbs.Item as={Link} to={`/${encRepo}/tree/${encBranch}`}>
                  {repo}
                </Breadcrumbs.Item>
                {pathParts.map((part, i) => (
                  <Breadcrumbs.Item
                    key={i}
                    as={Link}
                    to={buildPath(i)}
                    selected={i === pathParts.length - 1}
                  >
                    {part}
                  </Breadcrumbs.Item>
                ))}
              </Breadcrumbs>
            </div>
          )}

          {/* File table */}
          <div className="forge-card">
            {/* Latest commit header row — like GitHub */}
            {latestCommit && (
              <div className="forge-card-header" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '8px 16px' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '8px', flex: 1, minWidth: 0 }}>
                  {/* Avatar */}
                  <div className="avatar-circle avatar-circle-sm">
                    {latestCommit.author_name.charAt(0).toUpperCase()}
                  </div>
                  {/* Author */}
                  <strong style={{ fontSize: '13px', flexShrink: 0 }}>{latestCommit.author_name}</strong>
                  {/* Commit message (truncated) */}
                  <Link
                    to={`/${encRepo}/commit/${latestCommit.hash}`}
                    style={{
                      fontSize: '13px', color: 'var(--fg-default)', textDecoration: 'none',
                      overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                    }}
                    onMouseOver={e => e.currentTarget.style.color = 'var(--fg-accent)'}
                    onMouseOut={e => e.currentTarget.style.color = 'var(--fg-default)'}
                  >
                    {latestCommit.message}
                  </Link>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: '12px', flexShrink: 0, marginLeft: '12px' }}>
                  {/* Short hash */}
                  <Link
                    to={`/${encRepo}/commit/${latestCommit.hash}`}
                    className="text-mono"
                    style={{ fontSize: '12px', color: 'var(--fg-muted)', textDecoration: 'none' }}
                  >
                    {latestCommit.hash.slice(0, 7)}
                  </Link>
                  {/* Time */}
                  <span style={{ fontSize: '12px', color: 'var(--fg-muted)', whiteSpace: 'nowrap' }}>
                    {timeAgo(latestCommit.timestamp)}
                  </span>
                  {/* Commit count */}
                  <Link
                    to={`/${encRepo}/commits/${encBranch}`}
                    style={{ display: 'flex', alignItems: 'center', gap: '4px', fontSize: '13px', color: 'var(--fg-default)', textDecoration: 'none', whiteSpace: 'nowrap' }}
                  >
                    <HistoryIcon size={16} />
                    <strong>{commitCount}</strong> Commit{commitCount !== 1 ? 's' : ''}
                  </Link>
                </div>
              </div>
            )}

            {/* Go up row if in subfolder */}
            {path && (
              <Link
                to={pathParts.length > 1 ? buildPath(pathParts.length - 2) : `/${encRepo}/tree/${encBranch}`}
                className="file-row"
                style={{
                  display: 'grid', gridTemplateColumns: '20px 1fr',
                  padding: '6px 16px', borderBottom: '1px solid var(--border-muted)',
                  textDecoration: 'none', color: 'var(--fg-accent)', fontSize: '14px',
                }}
              >
                <span />
                <span>..</span>
              </Link>
            )}

            {/* File rows — 3 columns: icon+name, last commit msg, time ago */}
            {entries.map((entry, i) => (
              <div
                key={entry.name}
                className="file-row"
                style={{
                  display: 'grid',
                  gridTemplateColumns: '20px minmax(120px, 1fr) minmax(200px, 2fr) 100px',
                  gap: '8px',
                  alignItems: 'center',
                  padding: '6px 16px',
                  borderBottom: i < entries.length - 1 ? '1px solid var(--border-muted)' : 'none',
                  fontSize: '14px',
                }}
              >
                {/* Icon */}
                <span style={{ display: 'inline-flex', color: entry.kind === 'directory' ? 'var(--fg-accent)' : 'var(--fg-muted)' }}>
                  {entry.kind === 'directory' ? <FileDirectoryFillIcon size={16} /> : <FileIcon size={16} />}
                </span>

                {/* Name */}
                <Link
                  to={getEntryLink(entry)}
                  style={{
                    color: entry.kind === 'directory' ? 'var(--fg-accent)' : 'var(--fg-default)',
                    textDecoration: 'none',
                    overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                  }}
                  onMouseOver={e => e.currentTarget.style.textDecoration = 'underline'}
                  onMouseOut={e => e.currentTarget.style.textDecoration = 'none'}
                >
                  {entry.name}
                </Link>

                {/* Last commit message placeholder */}
                <span style={{ color: 'var(--fg-muted)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                </span>

                {/* Time ago placeholder */}
                <span style={{ color: 'var(--fg-muted)', textAlign: 'right', whiteSpace: 'nowrap', fontSize: '12px' }}>
                </span>
              </div>
            ))}

            {entries.length === 0 && (
              <div style={{ padding: '24px', textAlign: 'center', color: 'var(--fg-muted)' }}>
                This directory is empty.
              </div>
            )}
          </div>
        </div>

        {/* Right sidebar — About section */}
        {!path && repoInfo && (
          <div style={{ width: 296, flexShrink: 0 }}>
            {/* About */}
            <div style={{ marginBottom: '24px' }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: '12px' }}>
                <h3 style={{ fontSize: '16px', fontWeight: 600, margin: 0 }}>About</h3>
              </div>
              {repoInfo.description && (
                <p style={{ fontSize: '14px', color: 'var(--fg-default)', margin: '0 0 16px 0', lineHeight: 1.6 }}>
                  {repoInfo.description}
                </p>
              )}
              <div style={{ display: 'flex', flexDirection: 'column', gap: '8px', fontSize: '13px', color: 'var(--fg-muted)' }}>
                <span style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <GitBranchIcon size={16} />
                  {branches.length} branch{branches.length !== 1 ? 'es' : ''}
                </span>
                <span style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <HistoryIcon size={16} />
                  {commitCount} commit{commitCount !== 1 ? 's' : ''}
                </span>
                <span style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  <LockIcon size={16} />
                  Lock mode: {repoInfo.default_branch ? 'enabled' : 'disabled'}
                </span>
              </div>
            </div>

            {/* Clone URL */}
            <div style={{ marginBottom: '24px' }}>
              <h3 style={{ fontSize: '14px', fontWeight: 600, margin: '0 0 8px 0' }}>Clone</h3>
              <div style={{ display: 'flex', gap: '4px' }}>
                <TextInput value={cloneUrl} readOnly block monospace size="small" />
                <Button size="small" onClick={(e: React.MouseEvent) => {
                  e.preventDefault();
                  copyToClipboard(cloneUrl);
                }}>
                  <CopyIcon size={14} />
                </Button>
              </div>
            </div>

            {/* Languages placeholder */}
            <div>
              <h3 style={{ fontSize: '14px', fontWeight: 600, margin: '0 0 8px 0' }}>Activity</h3>
              <div style={{ fontSize: '13px', color: 'var(--fg-muted)' }}>
                {latestCommit && (
                  <span>Last updated {timeAgo(latestCommit.timestamp)}</span>
                )}
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
