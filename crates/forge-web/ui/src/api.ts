export interface User {
  username: string;
  is_admin: boolean;
}

export interface RepoInfo {
  name: string;
  description: string;
  created_at: number;
  branch_count: number;
  default_branch: string;
  last_commit_message: string;
  last_commit_author: string;
  last_commit_time: number;
}

export interface Branch {
  name: string;
  head: string;
}

export interface CommitSummary {
  hash: string;
  message: string;
  author_name: string;
  author_email: string;
  timestamp: number;
  parent_hashes: string[];
}

export interface CommitList {
  commits: CommitSummary[];
  total: number;
}

export interface TreeEntry {
  name: string;
  kind: 'file' | 'directory' | 'symlink';
  size: number;
  hash: string;
  asset_class?: string;
}

export interface TreeResponse {
  commit_hash: string;
  path: string;
  entries: TreeEntry[];
}

export interface AssetMetadata {
  asset_class: string;
  engine_version: string;
  package_flags: string[];
  dependencies: string[];
}

export interface FileContent {
  content: string | null;
  size: number;
  is_binary: boolean;
  hash: string;
  asset_metadata?: AssetMetadata | null;
}

export interface DiffFile {
  path: string;
  change_type: 'added' | 'modified' | 'deleted';
  old_size: number;
  new_size: number;
}

export interface CommitDetail {
  commit: CommitSummary | null;
  changes: DiffFile[];
}

export interface Lock {
  path: string;
  owner: string;
  workspace_id: string;
  created_at: number;
  reason: string;
}

export interface ServerInfo {
  version: string;
  uptime_secs: number;
  total_objects: number;
  total_size_bytes: number;
  branches: string[];
  active_locks: number;
}

// ── Actions types ──

export interface WorkflowInfo {
  id: number;
  name: string;
  yaml: string;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface RunInfo {
  id: number;
  workflow_id: number;
  workflow_name: string;
  trigger: string;
  trigger_ref: string;
  commit_hash: string;
  status: 'queued' | 'running' | 'success' | 'failure' | 'cancelled';
  started_at: number;
  finished_at: number;
  created_at: number;
  triggered_by: string;
}

export interface StepInfo {
  id: number;
  job_name: string;
  step_index: number;
  name: string;
  status: string;
  exit_code: number;
  log: string;
  started_at: number;
  finished_at: number;
}

export interface ArtifactListInfo {
  id: number;
  run_id: number;
  name: string;
  size_bytes: number;
  created_at: number;
}

export interface RunDetail {
  run: RunInfo | null;
  steps: StepInfo[];
  artifacts: ArtifactListInfo[];
}

export interface ReleaseInfo {
  id: number;
  tag: string;
  name: string;
  run_id: number;
  created_at: number;
  artifacts: ArtifactListInfo[];
}

// ── Issues & Pull Requests ──

export interface IssueInfo {
  id: number;
  title: string;
  body: string;
  author: string;
  status: string;
  labels: string[];
  assignee: string;
  created_at: number;
  updated_at: number;
  comment_count: number;
}

export interface IssueListResponse {
  issues: IssueInfo[];
  total: number;
  open_count: number;
  closed_count: number;
}

export interface PullRequestInfo {
  id: number;
  title: string;
  body: string;
  author: string;
  status: string;
  source_branch: string;
  target_branch: string;
  labels: string[];
  assignee: string;
  created_at: number;
  updated_at: number;
  comment_count: number;
}

export interface PullRequestListResponse {
  pull_requests: PullRequestInfo[];
  total: number;
  open_count: number;
  closed_count: number;
}

async function request<T>(url: string, options?: RequestInit): Promise<T> {
  const res = await fetch(url, {
    credentials: 'same-origin',
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new Error(`${res.status}: ${text || res.statusText}`);
  }
  if (res.status === 204) return undefined as T;
  return res.json();
}

const api = {
  // Auth
  login(username: string, password: string) {
    return request<{ ok: boolean }>('/api/auth/login', {
      method: 'POST',
      body: JSON.stringify({ username, password }),
    });
  },
  logout() {
    return request<void>('/api/auth/logout', { method: 'POST' });
  },
  me() {
    return request<User>('/api/auth/me').catch(() => null);
  },

  // Repos
  listRepos() {
    return request<RepoInfo[]>('/api/repos');
  },
  createRepo(name: string, description: string) {
    return request<{ success: boolean }>('/api/repos', {
      method: 'POST',
      body: JSON.stringify({ name, description }),
    });
  },

  // Branches
  listBranches(repo: string) {
    return request<Branch[]>(`/api/repos/${enc(repo)}/branches`);
  },

  // Commits
  listCommits(repo: string, branch: string, page = 1, limit = 30) {
    const offset = (page - 1) * limit;
    return request<CommitList>(`/api/repos/${enc(repo)}/commits/${enc(branch)}?limit=${limit}&offset=${offset}`);
  },

  // Tree browsing
  getTree(repo: string, branch: string, path = '') {
    const q = path ? `?path=${encodeURIComponent(path)}` : '';
    return request<TreeResponse>(`/api/repos/${enc(repo)}/tree/${enc(branch)}${q}`);
  },

  // File content
  getFile(repo: string, branch: string, path: string) {
    return request<FileContent>(`/api/repos/${enc(repo)}/blob/${enc(branch)}?path=${encodeURIComponent(path)}`);
  },

  // Commit detail
  getCommit(repo: string, hash: string) {
    return request<CommitDetail>(`/api/repos/${enc(repo)}/commit/${hash}`);
  },

  // Locks
  getLocks(repo: string) {
    return request<Lock[]>(`/api/repos/${enc(repo)}/locks`);
  },
  unlockFile(repo: string, path: string, force = false) {
    return request<void>(`/api/repos/${enc(repo)}/locks/${encodeURIComponent(path)}?owner=web-admin&force=${force}`, {
      method: 'DELETE',
    });
  },

  // Repo management
  updateRepo(repo: string, data: { new_name?: string; description?: string }): Promise<{ success: boolean }> {
    return request(`/api/repos/${enc(repo)}`, { method: 'PUT', body: JSON.stringify(data) });
  },
  deleteRepo(repo: string): Promise<{ success: boolean }> {
    return request(`/api/repos/${enc(repo)}`, { method: 'DELETE' });
  },

  // Server info
  getServerInfo() {
    return request<ServerInfo>('/api/server/info');
  },

  // ── Actions ──

  listWorkflows(repo: string) {
    return request<WorkflowInfo[]>(`/api/repos/${enc(repo)}/workflows`);
  },
  createWorkflow(repo: string, name: string, yaml: string) {
    return request<{ success: boolean; id?: number }>(`/api/repos/${enc(repo)}/workflows`, {
      method: 'POST', body: JSON.stringify({ name, yaml }),
    });
  },
  updateWorkflow(repo: string, id: number, data: { name?: string; yaml?: string; enabled?: boolean }) {
    return request<{ success: boolean }>(`/api/repos/${enc(repo)}/workflows/${id}`, {
      method: 'PUT', body: JSON.stringify(data),
    });
  },
  deleteWorkflow(repo: string, id: number) {
    return request<{ success: boolean }>(`/api/repos/${enc(repo)}/workflows/${id}`, { method: 'DELETE' });
  },
  triggerWorkflow(repo: string, workflowId: number, refName = 'refs/heads/main') {
    return request<{ success: boolean; run_id?: number }>(`/api/repos/${enc(repo)}/workflows/${workflowId}/trigger`, {
      method: 'POST', body: JSON.stringify({ ref_name: refName, triggered_by: 'web-user' }),
    });
  },
  listRuns(repo: string, workflowId = 0, limit = 50, offset = 0) {
    return request<{ runs: RunInfo[]; total: number }>(`/api/repos/${enc(repo)}/runs?workflow_id=${workflowId}&limit=${limit}&offset=${offset}`);
  },
  getRun(repo: string, runId: number) {
    return request<RunDetail>(`/api/repos/${enc(repo)}/runs/${runId}`);
  },
  cancelRun(repo: string, runId: number) {
    return request<{ success: boolean }>(`/api/repos/${enc(repo)}/runs/${runId}/cancel`, { method: 'POST' });
  },
  listReleases(repo: string) {
    return request<ReleaseInfo[]>(`/api/repos/${enc(repo)}/releases`);
  },

  // ── Issues ──
  listIssues(repo: string, status = '', limit = 50, offset = 0) {
    return request<IssueListResponse>(`/api/repos/${enc(repo)}/issues?status=${status}&limit=${limit}&offset=${offset}`);
  },
  getIssue(repo: string, id: number) {
    return request<IssueInfo>(`/api/repos/${enc(repo)}/issues/${id}`);
  },
  createIssue(repo: string, title: string, body = '', labels: string[] = []) {
    return request<{ success: boolean; id: number }>(`/api/repos/${enc(repo)}/issues`, {
      method: 'POST', body: JSON.stringify({ title, body, labels }),
    });
  },
  updateIssue(repo: string, id: number, data: { title?: string; body?: string; status?: string; labels?: string[]; assignee?: string }) {
    return request<{ success: boolean }>(`/api/repos/${enc(repo)}/issues/${id}`, {
      method: 'PUT', body: JSON.stringify(data),
    });
  },

  // ── Pull Requests ──
  listPullRequests(repo: string, status = '', limit = 50, offset = 0) {
    return request<PullRequestListResponse>(`/api/repos/${enc(repo)}/pulls?status=${status}&limit=${limit}&offset=${offset}`);
  },
  getPullRequest(repo: string, id: number) {
    return request<PullRequestInfo>(`/api/repos/${enc(repo)}/pulls/${id}`);
  },
  createPullRequest(repo: string, title: string, sourceBranch: string, targetBranch = 'main', body = '', labels: string[] = []) {
    return request<{ success: boolean; id: number }>(`/api/repos/${enc(repo)}/pulls`, {
      method: 'POST', body: JSON.stringify({ title, body, source_branch: sourceBranch, target_branch: targetBranch, labels }),
    });
  },
  mergePullRequest(repo: string, id: number) {
    return request<{ success: boolean; error: string }>(`/api/repos/${enc(repo)}/pulls/${id}/merge`, { method: 'POST' });
  },
  updatePullRequest(repo: string, id: number, data: { title?: string; body?: string; status?: string; labels?: string[]; assignee?: string }) {
    return request<{ success: boolean }>(`/api/repos/${enc(repo)}/pulls/${id}`, {
      method: 'PUT', body: JSON.stringify(data),
    });
  },
};

function enc(s: string) {
  return encodeURIComponent(s);
}

export default api;

/** Clipboard write with fallback for non-HTTPS (e.g. LAN access) */
export function copyToClipboard(text: string): Promise<void> {
  if (navigator.clipboard && window.isSecureContext) {
    return navigator.clipboard.writeText(text);
  }
  // Fallback: textarea trick
  const ta = document.createElement('textarea');
  ta.value = text;
  ta.style.position = 'fixed';
  ta.style.left = '-9999px';
  document.body.appendChild(ta);
  ta.select();
  document.execCommand('copy');
  document.body.removeChild(ta);
  return Promise.resolve();
}
