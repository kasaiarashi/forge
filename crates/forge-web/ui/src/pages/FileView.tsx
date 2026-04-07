import { useEffect, useState, useMemo } from 'react';
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
  GearIcon,
  RepoIcon,
} from '@primer/octicons-react';
import hljs from 'highlight.js';
import hljsLight from 'highlight.js/styles/github.css?url';
import hljsDark from 'highlight.js/styles/github-dark.css?url';
import type { FileContent } from '../api';
import api, { copyToClipboard } from '../api';
import { useTheme } from '../context/ThemeContext';

const extToLang: Record<string, string> = {
  ts: 'typescript',
  tsx: 'typescript',
  js: 'javascript',
  jsx: 'javascript',
  rs: 'rust',
  cpp: 'cpp',
  cc: 'cpp',
  cxx: 'cpp',
  h: 'cpp',
  hpp: 'cpp',
  cs: 'csharp',
  py: 'python',
  json: 'json',
  toml: 'toml',
  ini: 'ini',
  xml: 'xml',
  yaml: 'yaml',
  yml: 'yaml',
  md: 'markdown',
  css: 'css',
  html: 'html',
  htm: 'html',
  proto: 'protobuf',
  sql: 'sql',
  sh: 'bash',
  bash: 'bash',
  zsh: 'bash',
  go: 'go',
  java: 'java',
  rb: 'ruby',
  php: 'php',
  swift: 'swift',
  kt: 'kotlin',
  lua: 'lua',
  r: 'r',
  dockerfile: 'dockerfile',
  makefile: 'makefile',
};

function getLanguage(filename: string): string | null {
  const ext = filename.split('.').pop()?.toLowerCase() || '';
  const baseName = filename.toLowerCase();
  if (baseName === 'dockerfile') return 'dockerfile';
  if (baseName === 'makefile' || baseName === 'gnumakefile') return 'makefile';
  return extToLang[ext] || null;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export default function FileView() {
  const { repo = '', branch = 'main', '*': filePath = '' } = useParams();
  const { resolvedMode } = useTheme();
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

  useEffect(() => {
    const id = 'hljs-theme';
    let link = document.getElementById(id) as HTMLLinkElement | null;
    const href = resolvedMode === 'night' ? hljsDark : hljsLight;
    if (!link) {
      link = document.createElement('link');
      link.id = id;
      link.rel = 'stylesheet';
      document.head.appendChild(link);
    }
    link.href = href;
  }, [resolvedMode]);

  const pathParts = filePath.split('/');
  const fileName = pathParts[pathParts.length - 1];

  const highlightedLines = useMemo(() => {
    if (!file?.content) return [];
    const lang = getLanguage(fileName);
    let highlighted: string;
    try {
      if (lang) {
        highlighted = hljs.highlight(file.content, { language: lang }).value;
      } else {
        highlighted = hljs.highlightAuto(file.content).value;
      }
    } catch {
      highlighted = file.content
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
    }
    return highlighted.split('\n');
  }, [file?.content, fileName]);

  const buildBreadcrumbPath = (index: number): string => {
    if (index < pathParts.length - 1) {
      const parts = pathParts.slice(0, index + 1).join('/');
      return `/${encRepo}/tree/${encBranch}/${parts}`;
    }
    return `/${encRepo}/blob/${encBranch}/${filePath}`;
  };

  const handleCopy = () => {
    if (file?.content) {
      copyToClipboard(file.content);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
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
        <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}>
          <RepoIcon size={20} />
        </span>
        <Link
          to={`/${encRepo}`}
          style={{ fontSize: '20px', fontWeight: 600, color: 'var(--fg-accent)', textDecoration: 'none' }}
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
        <UnderlineNav.Item as={Link} to={`/${encRepo}/settings`} icon={GearIcon}>
          Settings
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
            <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><FileIcon size={16} /></span>
            <span style={{ fontWeight: 600, fontSize: '14px' }}>{fileName}</span>
            {file && (
              <>
                <Label variant="secondary" size="small">
                  {formatSize(file.size)}
                </Label>
                <span className="text-mono" style={{ fontSize: '12px', color: 'var(--fg-muted)' }}>
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
              href={`/api/repos/${encRepo}/raw/${encBranch}?path=${encodeURIComponent(filePath)}`}
            >
              Raw
            </Button>
          </div>
        </div>

        {/* UE Asset metadata panel */}
        {file?.asset_metadata && (
          <div style={{
            padding: '12px 16px',
            borderBottom: '1px solid var(--border-default)',
            backgroundColor: 'var(--bg-subtle)',
            fontSize: '13px',
          }}>
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: '16px', alignItems: 'center' }}>
              {file.asset_metadata.asset_class && (
                <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                  <span style={{ color: 'var(--fg-muted)' }}>Class:</span>
                  <span style={{
                    padding: '2px 8px',
                    borderRadius: '12px',
                    backgroundColor: 'var(--bg-accent-emphasis, #1f6feb)',
                    color: '#fff',
                    fontWeight: 600,
                    fontSize: '12px',
                  }}>
                    {file.asset_metadata.asset_class}
                  </span>
                </div>
              )}
              {file.asset_metadata.engine_version && (
                <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
                  <span style={{ color: 'var(--fg-muted)' }}>Engine:</span>
                  <span style={{ fontWeight: 500 }}>UE {file.asset_metadata.engine_version}</span>
                </div>
              )}
              {file.asset_metadata.package_flags.length > 0 && (
                <div style={{ display: 'flex', alignItems: 'center', gap: '4px', flexWrap: 'wrap' }}>
                  <span style={{ color: 'var(--fg-muted)' }}>Flags:</span>
                  {file.asset_metadata.package_flags.map((flag: string) => (
                    <span key={flag} style={{
                      padding: '1px 6px',
                      borderRadius: '8px',
                      border: '1px solid var(--border-default)',
                      fontSize: '11px',
                      color: 'var(--fg-muted)',
                    }}>
                      {flag}
                    </span>
                  ))}
                </div>
              )}
            </div>
            {file.asset_metadata.dependencies.length > 0 && (
              <details style={{ marginTop: '8px' }}>
                <summary style={{ cursor: 'pointer', color: 'var(--fg-muted)', fontSize: '12px' }}>
                  Dependencies ({file.asset_metadata.dependencies.length} packages)
                </summary>
                <ul style={{ margin: '4px 0 0 0', padding: '0 0 0 20px', fontSize: '12px', color: 'var(--fg-muted)' }}>
                  {file.asset_metadata.dependencies.map((dep: string) => (
                    <li key={dep}>{dep}</li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        )}

        {/* File content */}
        {file?.is_binary ? (
          <div style={{ padding: '24px', textAlign: 'center', color: 'var(--fg-muted)' }}>
            <div style={{ fontSize: '16px' }}>Binary file ({formatSize(file.size)})</div>
            <div style={{ marginTop: '8px' }}>
              <Button
                as="a"
                href={`/api/repos/${encRepo}/raw/${encBranch}?path=${encodeURIComponent(filePath)}`}
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
              {highlightedLines.map((lineHtml, i) => (
                <tr key={i}>
                  <td
                    className="line-number"
                    style={{
                      padding: '0 16px',
                      userSelect: 'none',
                      textAlign: 'right',
                      color: 'var(--fg-muted)',
                      verticalAlign: 'top',
                      width: 1,
                      whiteSpace: 'nowrap',
                    }}
                  >
                    {i + 1}
                  </td>
                  <td
                    style={{ padding: '0 16px', whiteSpace: 'pre', overflow: 'visible' }}
                    dangerouslySetInnerHTML={{ __html: lineHtml }}
                  />
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
