import { useEffect, useState } from 'react';
import { useParams, Link } from 'react-router-dom';
import {
  Breadcrumbs,
  Button,
  Spinner,
  Flash,
  Label,
  UnderlineNav,
} from '@primer/react';
import {
  FileIcon,
  CopyIcon,
  DownloadIcon,
  CodeIcon,
  GitCommitIcon,
  LockIcon,
  RepoIcon,
} from '@primer/octicons-react';
import type { FileContent } from '../api';
import api from '../api';

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export default function FileView() {
  const { repo = '', branch = 'main', '*': filePath = '' } = useParams();
  const [file, setFile] = useState<FileContent | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [copied, setCopied] = useState(false);

  const encRepo = encodeURIComponent(repo);
  const encBranch = encodeURIComponent(branch);

  useEffect(() => {
    setLoading(true);
    setError('');
    api
      .getFile(repo, branch, filePath)
      .then(setFile)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo, branch, filePath]);

  const pathParts = filePath.split('/');
  const fileName = pathParts[pathParts.length - 1];

  const buildBreadcrumbPath = (index: number): string => {
    if (index < pathParts.length - 1) {
      const parts = pathParts.slice(0, index + 1).join('/');
      return `/${encRepo}/tree/${encBranch}/${parts}`;
    }
    return `/${encRepo}/blob/${encBranch}/${filePath}`;
  };

  const handleCopy = () => {
    if (file?.content) {
      navigator.clipboard.writeText(file.content);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const lines = file?.content?.split('\n') || [];

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
          to={`/${encRepo}/tree/${encBranch}`}
          aria-current="page"
          icon={CodeIcon}
        >
          Code
        </UnderlineNav.Item>
        <UnderlineNav.Item
          as={Link}
          to={`/${encRepo}/commits/${encBranch}`}
          icon={GitCommitIcon}
        >
          Commits
        </UnderlineNav.Item>
        <UnderlineNav.Item as={Link} to={`/${encRepo}/locks`} icon={LockIcon}>
          Locks
        </UnderlineNav.Item>
      </UnderlineNav>

      {/* Breadcrumb */}
      <div style={{ margin: '16px 0' }}>
        <Breadcrumbs>
          <Breadcrumbs.Item as={Link} to={`/${encRepo}/tree/${encBranch}`}>
            root
          </Breadcrumbs.Item>
          {pathParts.map((part, i) => (
            <Breadcrumbs.Item
              key={i}
              as={Link}
              to={buildBreadcrumbPath(i)}
              selected={i === pathParts.length - 1}
            >
              {part}
            </Breadcrumbs.Item>
          ))}
        </Breadcrumbs>
      </div>

      {/* File container */}
      <div className="forge-card">
        {/* File header bar */}
        <div className="forge-card-header" style={{ justifyContent: 'space-between', flexWrap: 'wrap' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <span style={{ color: '#656d76', display: 'inline-flex' }}><FileIcon size={16} /></span>
            <span style={{ fontWeight: 600, fontSize: '14px' }}>{fileName}</span>
            {file && (
              <>
                <Label variant="secondary" size="small">
                  {formatSize(file.size)}
                </Label>
                <span className="text-mono" style={{ fontSize: '12px', color: '#656d76' }}>
                  {file.hash.slice(0, 8)}
                </span>
              </>
            )}
          </div>
          <div style={{ display: 'flex', gap: '4px' }}>
            <Button size="small" leadingVisual={CopyIcon} onClick={handleCopy}>
              {copied ? 'Copied!' : 'Copy'}
            </Button>
            <Button
              size="small"
              leadingVisual={DownloadIcon}
              as="a"
              href={`/api/repos/${encRepo}/blob/${encBranch}?path=${encodeURIComponent(filePath)}`}
            >
              Raw
            </Button>
          </div>
        </div>

        {/* File content */}
        {file?.is_binary ? (
          <div style={{ padding: '24px', textAlign: 'center', color: '#656d76' }}>
            <div style={{ fontSize: '16px' }}>Binary file ({formatSize(file.size)})</div>
            <div style={{ marginTop: '8px' }}>
              <Button
                as="a"
                href={`/api/repos/${encRepo}/blob/${encBranch}?path=${encodeURIComponent(filePath)}`}
              >
                Download
              </Button>
            </div>
          </div>
        ) : (
          <table style={{
            width: '100%',
            borderCollapse: 'collapse',
            fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace',
            fontSize: '12px',
            lineHeight: '20px',
          }}>
            <tbody>
              {lines.map((line, i) => (
                <tr key={i}>
                  <td
                    className="line-number"
                    style={{
                      padding: '0 16px',
                      userSelect: 'none',
                      textAlign: 'right',
                      color: '#656d76',
                      verticalAlign: 'top',
                      width: 1,
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {i + 1}
                  </td>
                  <td style={{ padding: '0 16px', whiteSpace: 'pre', overflow: 'visible' }}>
                    {line}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
