import { useEffect, useState } from 'react';
import { useParams, Link, useNavigate } from 'react-router-dom';
import {
  Breadcrumbs,
  ActionMenu,
  ActionList,
  Spinner,
  Flash,
  UnderlineNav,
} from '@primer/react';
import {
  FileDirectoryFillIcon,
  FileIcon,
  GitBranchIcon,
  CodeIcon,
  GitCommitIcon,
  LockIcon,
  ChevronRightIcon,
  RepoIcon,
} from '@primer/octicons-react';
import type { TreeEntry, Branch } from '../api';
import api from '../api';

function formatSize(bytes: number | null): string {
  if (bytes === null) return '';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
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

  useEffect(() => {
    setLoading(true);
    setError('');

    // If no branch in URL, we need to fetch branches first to find the default
    if (!branch) {
      api.listBranches(repo)
        .then((br) => {
          setBranches(br);
          const defaultBranch = br.find((b) => b.name === 'main') || br[0];
          if (defaultBranch) {
            const branchName = defaultBranch.name;
            setActiveBranch(branchName);
            return api.getTree(repo, branchName, '').then((tree) => {
              const sorted = [...tree.entries].sort((a, b) => {
                if (a.kind !== b.kind) return a.kind === 'directory' ? -1 : 1;
                return a.name.localeCompare(b.name);
              });
              setEntries(sorted);
            });
          }
          setEntries([]);
        })
        .catch((e) => setError(e.message))
        .finally(() => setLoading(false));
    } else {
      setActiveBranch(branch);
      Promise.all([api.getTree(repo, branch, path), api.listBranches(repo)])
        .then(([tree, br]) => {
          const sorted = [...tree.entries].sort((a, b) => {
            if (a.kind !== b.kind) return a.kind === 'directory' ? -1 : 1;
            return a.name.localeCompare(b.name);
          });
          setEntries(sorted);
          setBranches(br);
        })
        .catch((e) => setError(e.message))
        .finally(() => setLoading(false));
    }
  }, [repo, branch, path]);

  const pathParts = path ? path.split('/') : [];
  const encRepo = encodeURIComponent(repo);

  const buildPath = (index: number): string => {
    const parts = pathParts.slice(0, index + 1).join('/');
    return `/${encRepo}/tree/${encodeURIComponent(activeBranch)}/${parts}`;
  };

  const getEntryLink = (entry: TreeEntry): string => {
    const entryPath = path ? `${path}/${entry.name}` : entry.name;
    if (entry.kind === 'directory') {
      return `/${encRepo}/tree/${encodeURIComponent(activeBranch)}/${entryPath}`;
    }
    return `/${encRepo}/blob/${encodeURIComponent(activeBranch)}/${entryPath}`;
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
      {/* Repo name header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
        <span style={{ color: '#656d76', display: 'inline-flex' }}>
          <RepoIcon size={20} />
        </span>
        <Link
          to={`/${encRepo}`}
          style={{ fontSize: '20px', fontWeight: 600, color: '#0969da', textDecoration: 'none' }}
        >
          {repo}
        </Link>
      </div>

      {/* Repository tabs */}
      <UnderlineNav aria-label="Repository">
        <UnderlineNav.Item
          as={Link}
          to={`/${encRepo}/tree/${encodeURIComponent(activeBranch)}`}
          aria-current="page"
          icon={CodeIcon}
        >
          Code
        </UnderlineNav.Item>
        <UnderlineNav.Item
          as={Link}
          to={`/${encRepo}/commits/${encodeURIComponent(activeBranch)}`}
          icon={GitCommitIcon}
        >
          Commits
        </UnderlineNav.Item>
        <UnderlineNav.Item as={Link} to={`/${encRepo}/locks`} icon={LockIcon}>
          Locks
        </UnderlineNav.Item>
      </UnderlineNav>

      {/* Branch selector + breadcrumb row */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', margin: '16px 0', flexWrap: 'wrap' }}>
        <ActionMenu>
          <ActionMenu.Button leadingVisual={GitBranchIcon}>
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

        {pathParts.length > 0 && (
          <Breadcrumbs>
            <Breadcrumbs.Item as={Link} to={`/${encRepo}/tree/${encodeURIComponent(activeBranch)}`}>
              root
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
        )}
      </div>

      {/* File table */}
      <div className="forge-card">
        {/* Table header */}
        <div className="forge-card-header">
          <span style={{ color: '#656d76', display: 'inline-flex' }}><GitBranchIcon size={16} /></span>
          <span style={{ fontWeight: 600, fontSize: '14px' }}>{activeBranch}</span>
          {path && (
            <>
              <span style={{ color: '#656d76', display: 'inline-flex' }}><ChevronRightIcon size={12} /></span>
              <span style={{ fontSize: '14px', color: '#656d76' }}>{path}</span>
            </>
          )}
        </div>

        {/* Go up row if in subfolder */}
        {path && (
          <Link
            to={
              pathParts.length > 1
                ? buildPath(pathParts.length - 2)
                : `/${encRepo}/tree/${encodeURIComponent(activeBranch)}`
            }
            className="file-row"
            style={{
              display: 'flex',
              alignItems: 'center',
              padding: '6px 16px',
              borderBottom: '1px solid #d8dee4',
              textDecoration: 'none',
              color: '#0969da',
              fontSize: '14px',
            }}
          >
            ..
          </Link>
        )}

        {/* File rows */}
        {entries.map((entry, i) => (
          <Link
            key={entry.name}
            to={getEntryLink(entry)}
            className="file-row"
            style={{
              display: 'grid',
              gridTemplateColumns: '20px 1fr auto',
              gap: '8px',
              alignItems: 'center',
              padding: '6px 16px',
              borderBottom: i < entries.length - 1 ? '1px solid #d8dee4' : 'none',
              textDecoration: 'none',
              color: '#1f2328',
              fontSize: '14px',
            }}
          >
            {/* Icon */}
            <span style={{ display: 'inline-flex', color: entry.kind === 'directory' ? '#54aeff' : '#656d76' }}>
              {entry.kind === 'directory' ? (
                <FileDirectoryFillIcon size={16} />
              ) : (
                <FileIcon size={16} />
              )}
            </span>

            {/* Name */}
            <span style={{
              color: entry.kind === 'directory' ? '#0969da' : '#1f2328',
              fontWeight: entry.kind === 'directory' ? 600 : 400,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
            }}>
              {entry.name}
            </span>

            {/* Size */}
            <span style={{
              color: '#656d76',
              whiteSpace: 'nowrap',
              textAlign: 'right',
              minWidth: 60,
            }}>
              {entry.kind === 'file' ? formatSize(entry.size) : ''}
            </span>
          </Link>
        ))}

        {entries.length === 0 && (
          <div style={{ padding: '24px', textAlign: 'center', color: '#656d76' }}>
            This directory is empty.
          </div>
        )}
      </div>
    </div>
  );
}
