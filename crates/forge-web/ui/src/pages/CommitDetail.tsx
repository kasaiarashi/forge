import { useEffect, useState, useCallback } from 'react';
import { useParams, Link } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import {
  Spinner,
  Flash,
  Label,
} from '@primer/react';
import {
  GitCommitIcon,
  DiffAddedIcon,
  DiffRemovedIcon,
  DiffModifiedIcon,
  FileIcon,
  CopyIcon,
  ChevronDownIcon,
  ChevronRightIcon,
} from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import type { CommitDetail as CommitDetailType, DiffFile } from '../api';
import api, { repoPath,  copyToClipboard } from '../api';

function timeAgo(epoch: number): string {
  const date = new Date(epoch * 1000);
  const now = new Date();
  const seconds = Math.floor((now.getTime() - date.getTime()) / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} minute${minutes > 1 ? 's' : ''} ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hour${hours > 1 ? 's' : ''} ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days} day${days > 1 ? 's' : ''} ago`;
  const months = Math.floor(days / 30);
  return `${months} month${months > 1 ? 's' : ''} ago`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function StatusIcon({ status }: { status: DiffFile['change_type'] }) {
  const colorMap: Record<string, string> = {
    added: 'var(--fg-success)',
    deleted: 'var(--fg-danger)',
    modified: 'var(--fg-warning)',
  };
  const color = colorMap[status] || 'var(--fg-warning)';
  const iconMap: Record<string, typeof DiffModifiedIcon> = {
    added: DiffAddedIcon,
    deleted: DiffRemovedIcon,
    modified: DiffModifiedIcon,
  };
  const Icon = iconMap[status] || DiffModifiedIcon;
  return (
    <span style={{ color, display: 'inline-flex', flexShrink: 0 }}>
      <Icon size={16} />
    </span>
  );
}

/** Compute unified diff between old and new text using LCS. */
function computeDiff(oldText: string, newText: string): { tag: 'equal' | 'add' | 'del'; line: string }[] {
  // Normalize line endings to LF.
  const oldLines = oldText.replace(/\r\n/g, '\n').replace(/\r/g, '\n').split('\n');
  const newLines = newText.replace(/\r\n/g, '\n').replace(/\r/g, '\n').split('\n');

  // Remove trailing empty line from split (artifact of trailing newline).
  if (oldLines.length > 0 && oldLines[oldLines.length - 1] === '') oldLines.pop();
  if (newLines.length > 0 && newLines[newLines.length - 1] === '') newLines.pop();

  const result: { tag: 'equal' | 'add' | 'del'; line: string }[] = [];

  // Too large — show as full replacement.
  if (oldLines.length > 5000 || newLines.length > 5000) {
    for (const l of oldLines) result.push({ tag: 'del', line: l });
    for (const l of newLines) result.push({ tag: 'add', line: l });
    return result;
  }

  // Build LCS table (Hunt-McIlroy style with O(NM) DP for correctness).
  const n = oldLines.length, m = newLines.length;

  // For moderate sizes, use full DP. For very large, fall back to line-hash matching.
  if (n * m > 2_000_000) {
    // Hash-based approach: match identical lines by position.
    const oldSet = new Map<string, number[]>();
    oldLines.forEach((l, i) => { const arr = oldSet.get(l) || []; arr.push(i); oldSet.set(l, arr); });

    let oi = 0, ni = 0;
    while (oi < n && ni < m) {
      if (oldLines[oi] === newLines[ni]) {
        result.push({ tag: 'equal', line: oldLines[oi] });
        oi++; ni++;
      } else {
        // Look ahead in new for a match with current old.
        let foundNew = -1;
        for (let j = ni + 1; j < Math.min(ni + 10, m); j++) {
          if (newLines[j] === oldLines[oi]) { foundNew = j; break; }
        }
        let foundOld = -1;
        for (let j = oi + 1; j < Math.min(oi + 10, n); j++) {
          if (oldLines[j] === newLines[ni]) { foundOld = j; break; }
        }

        if (foundOld >= 0 && (foundNew < 0 || foundOld - oi <= foundNew - ni)) {
          while (oi < foundOld) { result.push({ tag: 'del', line: oldLines[oi++] }); }
        } else if (foundNew >= 0) {
          while (ni < foundNew) { result.push({ tag: 'add', line: newLines[ni++] }); }
        } else {
          result.push({ tag: 'del', line: oldLines[oi++] });
          result.push({ tag: 'add', line: newLines[ni++] });
        }
      }
    }
    while (oi < n) result.push({ tag: 'del', line: oldLines[oi++] });
    while (ni < m) result.push({ tag: 'add', line: newLines[ni++] });
    return result;
  }

  // Standard LCS DP.
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = 1; i <= n; i++) {
    for (let j = 1; j <= m; j++) {
      if (oldLines[i - 1] === newLines[j - 1]) {
        dp[i][j] = dp[i - 1][j - 1] + 1;
      } else {
        dp[i][j] = Math.max(dp[i - 1][j], dp[i][j - 1]);
      }
    }
  }

  // Backtrack.
  let i = n, j = m;
  const edits: { tag: 'equal' | 'add' | 'del'; line: string }[] = [];
  while (i > 0 && j > 0) {
    if (oldLines[i - 1] === newLines[j - 1]) {
      edits.push({ tag: 'equal', line: oldLines[i - 1] });
      i--; j--;
    } else if (dp[i - 1][j] >= dp[i][j - 1]) {
      edits.push({ tag: 'del', line: oldLines[i - 1] });
      i--;
    } else {
      edits.push({ tag: 'add', line: newLines[j - 1] });
      j--;
    }
  }
  while (i > 0) { edits.push({ tag: 'del', line: oldLines[--i] }); }
  while (j > 0) { edits.push({ tag: 'add', line: newLines[--j] }); }

  edits.reverse();
  return edits;
}

interface FileDiffViewProps {
  repo: string;
  commitHash: string;
  parentHash: string | null;
  file: DiffFile;
}

function FileDiffView({ repo, commitHash, parentHash, file }: FileDiffViewProps) {
  const [diffLines, setDiffLines] = useState<{ tag: string; line: string }[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  useEffect(() => {
    const load = async () => {
      try {
        setLoading(true);
        let oldContent = '';
        let newContent = '';

        if (file.change_type === 'added') {
          const resp = await api.getFile(repo, commitHash, file.path);
          if (resp.is_binary) { setDiffLines([{ tag: 'equal', line: 'Binary file' }]); return; }
          newContent = resp.content || '';
        } else if (file.change_type === 'deleted') {
          if (parentHash) {
            const resp = await api.getFile(repo, parentHash, file.path);
            if (resp.is_binary) { setDiffLines([{ tag: 'equal', line: 'Binary file' }]); return; }
            oldContent = resp.content || '';
          }
        } else {
          // Modified
          const [newResp, oldResp] = await Promise.all([
            api.getFile(repo, commitHash, file.path),
            parentHash ? api.getFile(repo, parentHash, file.path).catch(() => ({ content: '', is_binary: false })) : Promise.resolve({ content: '', is_binary: false }),
          ]);
          if (newResp.is_binary || oldResp.is_binary) {
            setDiffLines([{ tag: 'equal', line: `Binary file (${formatBytes(file.old_size)} → ${formatBytes(file.new_size)})` }]);
            return;
          }
          oldContent = oldResp.content || '';
          newContent = newResp.content || '';
        }

        const lines = computeDiff(oldContent, newContent);
        setDiffLines(lines);
      } catch (e: any) {
        setError(e.message);
      } finally {
        setLoading(false);
      }
    };
    load();
  }, [repo, commitHash, parentHash, file]);

  if (loading) return <div style={{ padding: '16px', textAlign: 'center' }}><Spinner size="small" /></div>;
  if (error) return <div style={{ padding: '8px 16px', color: 'var(--fg-danger)', fontSize: '13px' }}>{error}</div>;
  if (!diffLines) return null;

  // Show with context: collapse long runs of equal lines
  const contextLines = 3;
  const chunks: { start: number; end: number }[] = [];
  let i = 0;
  while (i < diffLines.length) {
    if (diffLines[i].tag !== 'equal') {
      const start = Math.max(0, i - contextLines);
      let end = i;
      while (end < diffLines.length && diffLines[end].tag !== 'equal') end++;
      end = Math.min(diffLines.length, end + contextLines);
      // Merge with previous chunk if overlapping
      if (chunks.length > 0 && start <= chunks[chunks.length - 1].end) {
        chunks[chunks.length - 1].end = end;
      } else {
        chunks.push({ start, end });
      }
      i = end;
    } else {
      i++;
    }
  }

  // If no changes visible (all equal), show a message
  if (chunks.length === 0) {
    return <div style={{ padding: '8px 16px', color: 'var(--fg-muted)', fontSize: '13px', fontStyle: 'italic' }}>No visible text changes</div>;
  }

  return (
    <div style={{
      fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace',
      fontSize: '12px',
      lineHeight: '20px',
      overflow: 'auto',
      maxHeight: '600px',
    }}>
      {chunks.map((chunk, ci) => (
        <div key={ci}>
          {ci > 0 && (
            <div style={{ padding: '4px 16px', backgroundColor: 'var(--bg-inset)', color: 'var(--fg-muted)', textAlign: 'center', fontSize: '11px', borderTop: '1px solid var(--border-muted)', borderBottom: '1px solid var(--border-muted)' }}>
              ···
            </div>
          )}
          {diffLines.slice(chunk.start, chunk.end).map((line, li) => {
            const bg = line.tag === 'add' ? 'rgba(46,160,67,0.15)' : line.tag === 'del' ? 'rgba(248,81,73,0.15)' : 'transparent';
            const color = line.tag === 'add' ? 'var(--fg-success)' : line.tag === 'del' ? 'var(--fg-danger)' : 'var(--fg-default)';
            const prefix = line.tag === 'add' ? '+' : line.tag === 'del' ? '-' : ' ';
            return (
              <div key={`${ci}-${li}`} style={{ display: 'flex', backgroundColor: bg, minWidth: 'fit-content' }}>
                <span style={{ width: '20px', textAlign: 'center', color: 'var(--fg-muted)', userSelect: 'none', flexShrink: 0 }}>{prefix}</span>
                <pre style={{ margin: 0, padding: '0 8px', whiteSpace: 'pre', color }}>{line.line || ' '}</pre>
              </div>
            );
          })}
        </div>
      ))}
    </div>
  );
}

export default function CommitDetail() {
  const repo = useRepoParam();
  const { hash = '' } = useParams<{ hash?: string }>();
  const [commit, setCommit] = useState<CommitDetailType | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [copied, setCopied] = useState(false);
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set());

  const encRepo = repoPath(repo);

  useEffect(() => {
    setLoading(true);
    setError('');
    setExpandedFiles(new Set());
    api
      .getCommit(repo, hash)
      .then(setCommit)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo, hash]);

  const handleCopy = () => {
    if (commit?.commit) {
      copyToClipboard(commit.commit.hash);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const toggleFile = useCallback((path: string) => {
    setExpandedFiles(prev => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

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

  if (!commit || !commit.commit) return null;

  const info = commit.commit;
  const files = commit.changes;
  const parentHash = info.parent_hashes.length > 0 ? info.parent_hashes[0] : null;

  return (
    <div>
      <RepoHeader repo={repo} currentTab="commits" />

      {/* Commit header */}
      <div className="forge-card" style={{ marginBottom: '16px' }}>
        <div style={{
          background: 'var(--bg-subtle)',
          padding: '16px',
          borderBottom: '1px solid var(--border-default)',
        }}>
          <h2 style={{ fontSize: '20px', fontWeight: 600, margin: '0 0 8px 0', wordBreak: 'break-word' }}>
            {info.message}
          </h2>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', flexWrap: 'wrap' }}>
            <div className="avatar-circle avatar-circle-sm">
              {info.author_name.charAt(0).toUpperCase()}
            </div>
            <span style={{ fontWeight: 600, fontSize: '14px' }}>{info.author_name}</span>
            <span style={{ color: 'var(--fg-muted)', fontSize: '14px' }}>
              committed {timeAgo(info.timestamp)}
            </span>
          </div>
        </div>

        <div style={{ padding: '8px 16px' }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexWrap: 'wrap', gap: '8px' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><GitCommitIcon size={16} /></span>
              <span style={{ fontSize: '14px', color: 'var(--fg-muted)' }}>Commit</span>
              <span className="text-mono" style={{ fontSize: '14px', fontWeight: 600 }}>{info.hash.slice(0, 7)}</span>
              <button onClick={handleCopy} style={{ background: 'none', border: 'none', cursor: 'pointer', padding: '2px', display: 'flex', alignItems: 'center', color: copied ? 'var(--fg-success)' : 'var(--fg-muted)' }}>
                <CopyIcon size={14} />
              </button>
            </div>
            {parentHash && (
              <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
                <span style={{ fontSize: '14px', color: 'var(--fg-muted)' }}>Parent:</span>
                <Link to={`/${encRepo}/commit/${parentHash}`} className="text-mono" style={{ fontSize: '14px', color: 'var(--fg-accent)', textDecoration: 'none' }}>
                  {parentHash.slice(0, 7)}
                </Link>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Diff stats */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
        <span style={{ color: 'var(--fg-muted)', display: 'inline-flex' }}><FileIcon size={16} /></span>
        <span style={{ fontWeight: 600, fontSize: '14px' }}>
          {files.length} file{files.length !== 1 ? 's' : ''} changed
        </span>
        <span style={{ color: 'var(--fg-muted)', fontSize: '13px' }}>
          (click a file to view diff)
        </span>
      </div>

      {/* File changes with expandable diffs */}
      <div className="forge-card">
        {files.map((file, i) => {
          const isExpanded = expandedFiles.has(file.path);
          return (
            <div key={file.path}>
              <div
                className="file-row"
                onClick={() => toggleFile(file.path)}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: '8px',
                  padding: '8px 16px',
                  borderBottom: (isExpanded || i < files.length - 1) ? '1px solid var(--border-muted)' : 'none',
                  cursor: 'pointer',
                  userSelect: 'none',
                }}
              >
                <span style={{ color: 'var(--fg-muted)', display: 'inline-flex', flexShrink: 0 }}>
                  {isExpanded ? <ChevronDownIcon size={16} /> : <ChevronRightIcon size={16} />}
                </span>
                <StatusIcon status={file.change_type} />
                <span className="text-mono" style={{ flex: 1, fontSize: '14px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {file.path}
                </span>
                {file.old_size > 0 && file.new_size > 0 && (
                  <span className="text-mono" style={{ fontSize: '12px', color: 'var(--fg-muted)', flexShrink: 0 }}>
                    {formatBytes(file.old_size)} → {formatBytes(file.new_size)}
                  </span>
                )}
                <Label size="small" variant={file.change_type === 'added' ? 'success' : file.change_type === 'deleted' ? 'danger' : 'attention'}>
                  {file.change_type}
                </Label>
              </div>
              {isExpanded && (
                <div style={{ borderBottom: i < files.length - 1 ? '1px solid var(--border-muted)' : 'none', backgroundColor: 'var(--bg-inset)' }}>
                  <FileDiffView repo={repo} commitHash={info.hash} parentHash={parentHash} file={file} />
                </div>
              )}
            </div>
          );
        })}

        {files.length === 0 && (
          <div style={{ padding: '24px', textAlign: 'center', color: 'var(--fg-muted)' }}>
            No files changed in this commit.
          </div>
        )}
      </div>
    </div>
  );
}
