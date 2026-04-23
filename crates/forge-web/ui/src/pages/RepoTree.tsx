import { useEffect, useState } from 'react';
import { useParams, Link, useNavigate } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import {
  Breadcrumbs,
  ActionMenu,
  ActionList,
  Spinner,
  Flash,
  TextInput,
  Button,
} from '@primer/react';
import {
  FileDirectoryFillIcon,
  FileIcon,
  GitBranchIcon,
  CodeIcon,
  CopyIcon,
  HistoryIcon,
  TagIcon,
  SearchIcon,
  LockIcon,
  GearIcon,
} from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import type { TreeEntry, Branch, CommitSummary, RepoInfo, LanguageStat } from '../api';
import api, { repoPath,  copyToClipboard, getLanguageStats } from '../api';
import { useAuth } from '../context/AuthContext';

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
  const repo = useRepoParam();
  const { branch, '*': pathStr = '' } = useParams<{ branch?: string; '*'?: string }>();
  const path = pathStr || '';
  const navigate = useNavigate();

  const [entries, setEntries] = useState<TreeEntry[]>([]);
  const [branches, setBranches] = useState<Branch[]>([]);
  const [activeBranch, setActiveBranch] = useState(branch || '');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  // True if the repo exists but has zero branches (no commits pushed yet).
  // We render the quickstart instructions instead of trying to load a tree.
  const [isEmpty, setIsEmpty] = useState(false);
  const [latestCommit, setLatestCommit] = useState<CommitSummary | null>(null);
  const [commitCount, setCommitCount] = useState(0);
  const [repoInfo, setRepoInfo] = useState<RepoInfo | null>(null);
  const [cloneCopied, setCloneCopied] = useState(false);
  const [showCloneMenu, setShowCloneMenu] = useState(false);
  const [languages, setLanguages] = useState<LanguageStat[]>([]);
  const [recentCommits, setRecentCommits] = useState<CommitSummary[]>([]);
  const { user } = useAuth();
  const [newBranchName, setNewBranchName] = useState('');
  const [showNewBranchInput, setShowNewBranchInput] = useState(false);
  const [isBranchLoading, setIsBranchLoading] = useState(false);

  // Bare server URL (for `forge remote add` etc.) and the full
  // server-plus-path URL the user will paste into `forge clone`. The full
  // form is the GitHub-style `http://host:9876/<owner>/<name>`.
  const serverUrl = `${window.location.protocol}//${window.location.hostname}:9876`;
  const cloneUrl = `${serverUrl}/${repo}`;

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
    setIsEmpty(false);

    const loadData = async () => {
      try {
        const [br, repos] = await Promise.all([
          api.listBranches(repo),
          api.listRepos(),
        ]);
        setBranches(br);
        const ri = repos.find(r => r.name === repo) || null;
        setRepoInfo(ri);

        // Empty repo: no branches yet → show the quickstart instructions
        // instead of trying to load a tree from a branch that doesn't exist.
        if (br.length === 0) {
          setIsEmpty(true);
          setLoading(false);
          return;
        }

        const resolvedBranch = branch || (br.find(b => b.name === 'main') || br[0])?.name || 'main';
        setActiveBranch(resolvedBranch);

        // Fetch critical data first (tree + latest commit), defer sidebar data.
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

        // Load sidebar data in background (non-blocking).
        getLanguageStats(repo, resolvedBranch).then(setLanguages).catch(() => {});
        api.listCommits(repo, resolvedBranch, 1, 3).then(r => setRecentCommits(r.commits)).catch(() => {});
      } catch (e: any) {
        setError(e.message);
      } finally {
        setLoading(false);
      }
    };

    loadData();
  }, [repo, branch, path]);

  const handleCreateBranch = async () => {
    if (!newBranchName.trim()) return;
    setIsBranchLoading(true);
    try {
      await api.createBranch(repo, newBranchName, activeBranch);
      setNewBranchName('');
      setShowNewBranchInput(false);
      navigate(`/${encRepo}/tree/${encodeURIComponent(newBranchName)}`);
    } catch (e: any) {
      setError(`Failed to create branch: ${e.message}`);
    } finally {
      setIsBranchLoading(false);
    }
  };

  const handleDeleteBranch = async (branchToDelete: string) => {
    if (!window.confirm(`Are you sure you want to delete branch '${branchToDelete}'?`)) return;
    setIsBranchLoading(true);
    try {
      await api.deleteBranch(repo, branchToDelete);
      if (branchToDelete === activeBranch) {
        navigate(`/${encRepo}`);
      } else {
        const br = await api.listBranches(repo);
        setBranches(br);
      }
    } catch (e: any) {
      setError(`Failed to delete branch: ${e.message}`);
    } finally {
      setIsBranchLoading(false);
    }
  };

  const pathParts = path ? path.split('/') : [];
  const encRepo = repoPath(repo);
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

  if (isEmpty) {
    // Quickstart instructions for an empty repo. Same shape as the
    // "Quick setup for {repo}" card in Dashboard.tsx so refresh / direct
    // navigation feels consistent. Both flows now use the GitHub-style
    // single-URL form.
    const repoName = repo.split('/').pop() || repo;
    const quickstartInit = [
      `forge clone ${cloneUrl}`,
      `cd ${repoName}`,
      'echo "# starter" > README.md',
      'forge add .',
      'forge commit -m "first commit"',
      'forge push',
    ];
    const quickstartPush = [
      `forge remote add origin ${cloneUrl}`,
      'forge push',
    ];
    return (
      <div>
        <RepoHeader repo={repo} currentTab="code" activeBranch="main" visibility={repoInfo?.visibility} />
        <div className="forge-card" style={{ marginTop: '16px' }}>
          <div className="forge-card-header" style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <CodeIcon size={16} />
            <span style={{ fontWeight: 600 }}>Quick setup — {repo} is empty</span>
          </div>
          <div style={{ padding: '16px' }}>
            <p style={{ color: 'var(--fg-muted)', fontSize: '14px', margin: '0 0 16px 0' }}>
              This repository has no commits yet. Create one from the command line:
            </p>

            <div style={{ marginBottom: '16px' }}>
              <div style={{ fontSize: '12px', color: 'var(--fg-muted)', marginBottom: '4px' }}>
                Create a new repository on the command line
              </div>
              <pre style={{ background: 'var(--bg-canvas-inset)', padding: '12px', borderRadius: '6px', fontSize: '12px', overflow: 'auto', margin: 0 }}>
{quickstartInit.join('\n')}
              </pre>
            </div>

            <div>
              <div style={{ fontSize: '12px', color: 'var(--fg-muted)', marginBottom: '4px' }}>
                …or push an existing repository
              </div>
              <pre style={{ background: 'var(--bg-canvas-inset)', padding: '12px', borderRadius: '6px', fontSize: '12px', overflow: 'auto', margin: 0 }}>
{quickstartPush.join('\n')}
              </pre>
            </div>
          </div>
        </div>
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
      <RepoHeader repo={repo} currentTab="code" activeBranch={activeBranch} visibility={repoInfo?.visibility} />

      {/* Main content: left file browser + right sidebar */}
      <div style={{ maxWidth: '1280px', margin: '0 auto', padding: '0 var(--space-6)' }}>
        <div style={{ display: 'flex', gap: '24px', marginTop: '16px' }}>
        {/* Left: file browser */}
        <div style={{ flex: 1, minWidth: 0 }}>

          {/* Top toolbar: branch selector, branches count, search, Code button */}
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px', flexWrap: 'wrap' }}>
            {/* Branch selector */}
            <ActionMenu>
              <ActionMenu.Button leadingVisual={GitBranchIcon} size="small" disabled={isBranchLoading}>
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
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', width: '100%' }}>
                        <span>{b.name}</span>
                        {b.name !== 'main' && (
                          <Button 
                            variant="invisible" 
                            size="small" 
                            onClick={(e) => {
                              e.stopPropagation();
                              handleDeleteBranch(b.name);
                            }}
                            style={{ color: 'var(--fg-danger)', padding: '0 4px', height: 'auto' }}
                            aria-label={`Delete branch ${b.name}`}
                          >
                            Delete
                          </Button>
                        )}
                      </div>
                    </ActionList.Item>
                  ))}
                  <ActionList.Divider />
                  {!showNewBranchInput ? (
                    <ActionList.Item onSelect={(e) => {
                      e.preventDefault();
                      setShowNewBranchInput(true);
                    }}>
                      <span style={{ color: 'var(--fg-accent)' }}>+ Create new branch</span>
                    </ActionList.Item>
                  ) : (
                    <div style={{ padding: '8px 16px', display: 'flex', gap: '8px', flexDirection: 'column' }}>
                      <span style={{ fontSize: '12px', color: 'var(--fg-muted)' }}>From {activeBranch}:</span>
                      <TextInput 
                        autoFocus
                        value={newBranchName}
                        onChange={(e) => setNewBranchName(e.target.value)}
                        placeholder="Branch name"
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') handleCreateBranch();
                          if (e.key === 'Escape') setShowNewBranchInput(false);
                        }}
                        block
                      />
                      <div style={{ display: 'flex', gap: '8px' }}>
                        <Button size="small" variant="primary" onClick={handleCreateBranch} flex={1}>Create</Button>
                        <Button size="small" onClick={() => setShowNewBranchInput(false)}>Cancel</Button>
                      </div>
                    </div>
                  )}
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
                  <div style={{ display: 'flex', gap: '4px', marginBottom: '4px' }}>
                    <TextInput value={`forge clone ${cloneUrl}`} readOnly block monospace size="small" />
                    <Button size="small" onClick={copyCloneUrl}>
                      {cloneCopied ? 'Copied!' : <CopyIcon size={16} />}
                    </Button>
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
              <div className="forge-card-header" style={{ justifyContent: 'space-between', borderBottom: '1px solid var(--border-muted)' }}>
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
                  display: 'grid', gridTemplateColumns: '24px 1fr', gap: 'var(--space-2)',
                  textDecoration: 'none', color: 'var(--fg-accent)', fontSize: '14px',
                }}
              >
                <span />
                <span>..</span>
              </Link>
            )}

            {/* File rows — 3 columns: icon+name, last commit msg, time ago */}
            {entries.map((entry) => (
              <div
                key={entry.name}
                className="file-row"
                style={{
                  display: 'grid',
                  gridTemplateColumns: '24px minmax(150px, 1.5fr) minmax(200px, 2fr) 120px',
                  gap: 'var(--space-2)',
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
                {entry.asset_class && (
                  <span style={{
                    display: 'inline-block',
                    marginLeft: '8px',
                    padding: '1px 6px',
                    fontSize: '11px',
                    borderRadius: '8px',
                    backgroundColor: 'var(--bg-accent-emphasis, #1f6feb)',
                    color: '#fff',
                    fontWeight: 500,
                    lineHeight: '16px',
                    whiteSpace: 'nowrap',
                    flexShrink: 0,
                  }}>
                    {entry.asset_class}
                  </span>
                )}

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
                {user?.is_admin && (
                  <Button variant="invisible" size="small" leadingVisual={GearIcon} style={{ color: 'var(--fg-muted)' }} />
                )}
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
                <TextInput value={`forge clone ${cloneUrl}`} readOnly block monospace size="small" />
                <Button size="small" onClick={copyCloneUrl}>
                  {cloneCopied ? 'Copied!' : <CopyIcon size={14} />}
                </Button>
              </div>
            </div>

            {/* Releases */}
            <div style={{ marginBottom: '24px' }}>
              <h3 style={{ fontSize: '14px', fontWeight: 600, margin: '0 0 8px 0' }}>Releases</h3>
              <div style={{ fontSize: '13px', color: 'var(--fg-muted)', marginBottom: '8px' }}>
                No releases published
              </div>
              <Link
                to={`/${encRepo}/releases`}
                style={{ fontSize: '13px', color: 'var(--fg-accent)', textDecoration: 'none' }}
                onMouseOver={e => e.currentTarget.style.textDecoration = 'underline'}
                onMouseOut={e => e.currentTarget.style.textDecoration = 'none'}
              >
                Create a new release
              </Link>
            </div>

            {/* Languages */}
            {languages.length > 0 && (
              <div style={{ marginBottom: '24px', borderTop: '1px solid var(--border-muted)', paddingTop: '16px' }}>
                <h3 style={{ fontSize: '14px', fontWeight: 600, margin: '0 0 8px 0' }}>Languages</h3>
                <div style={{ display: 'flex', height: '8px', borderRadius: '4px', overflow: 'hidden', marginBottom: '8px' }}>
                  {languages.map(lang => (
                    <div key={lang.name} style={{ width: `${lang.percentage}%`, backgroundColor: lang.color }} title={`${lang.name} ${lang.percentage.toFixed(1)}%`}></div>
                  ))}
                </div>
                <ul style={{ listStyle: 'none', padding: 0, margin: 0, fontSize: '12px', display: 'flex', flexWrap: 'wrap', gap: '8px' }}>
                  {languages.map(lang => (
                    <li key={lang.name} style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                      <span style={{ width: 8, height: 8, borderRadius: '50%', backgroundColor: lang.color, display: 'inline-block' }}></span>
                      <span style={{ fontWeight: 600 }}>{lang.name}</span>
                      <span style={{ color: 'var(--fg-muted)' }}>{lang.percentage.toFixed(1)}%</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}

            {/* Recent activity */}
            <div style={{ marginBottom: '24px', borderTop: '1px solid var(--border-muted)', paddingTop: '16px' }}>
              <h3 style={{ fontSize: '14px', fontWeight: 600, margin: '0 0 8px 0' }}>Recent activity</h3>
              {recentCommits.map(c => (
                <div key={c.hash} style={{ fontSize: '12px', marginBottom: '8px' }}>
                  <Link to={`/${repoPath(repo)}/commit/${c.hash}`} style={{ color: 'var(--fg-accent)', textDecoration: 'none' }}>
                    {c.message.length > 50 ? c.message.slice(0, 50) + '...' : c.message}
                  </Link>
                  <div style={{ color: 'var(--fg-muted)' }}>{c.author_name} · {timeAgo(c.timestamp)}</div>
                </div>
              ))}
            </div>
          </div>
        )}
        </div>
      </div>
    </div>
  );
}
