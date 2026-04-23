export interface User {
  username: string;
  is_admin: boolean;
}

/**
 * Server-side user record returned by `/api/auth/users` (admin only) and
 * other admin endpoints. Distinct from the lighter [`User`] which is the
 * shape `/api/auth/me` returns for the SPA's own session — `User` predates
 * the multi-user admin surface and we don't want to retrofit it.
 */
export interface UserSummary {
  id: number;
  username: string;
  email: string;
  display_name: string;
  is_server_admin: boolean;
}

export interface RepoMember {
  user: UserSummary;
  role: string; // "read" | "write" | "admin"
}

export interface PatInfo {
  id: number;
  name: string;
  user_id: number;
  scopes: string[];
  created_at: number;
  last_used_at: number;
  expires_at: number; // 0 = never
}

export interface CommentInfo {
  id: number;
  repo: string;
  issue_id: number;
  kind: string; // "issue" | "pull_request"
  author: string;
  body: string;
  created_at: number;
  updated_at: number;
}

export interface SessionInfo {
  id: number;
  user_id: number;
  created_at: number;
  last_used_at: number;
  expires_at: number;
  user_agent: string;
  ip: string;
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
  visibility: string; // "private" | "public"
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
  repos: string[];
  repo_count: number;
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

/**
 * Endpoints that we expect to be unauthenticated. A 401 from these is
 * informational (e.g. `me()` returning null when logged out) and should NOT
 * trigger an automatic redirect to /login. Everything else, on 401, hard-
 * navigates to /login because the user is no longer authenticated and the
 * page they're on can't render without data.
 */
const PUBLIC_AUTH_PATHS = new Set([
  '/api/auth/login',
  '/api/auth/me',
  '/api/auth/initialized',
  '/api/auth/bootstrap',
  '/api/auth/logout',
]);

async function request<T>(url: string, options?: RequestInit): Promise<T> {
  const res = await fetch(url, {
    credentials: 'same-origin',
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (!res.ok) {
    // 401 on a normal data endpoint means the session expired or was
    // revoked. Hard-redirect to /login so the user is never stranded on a
    // page rendering "401: invalid or expired session" as a flash error.
    // Skip the redirect for the auth endpoints themselves so the login form
    // and the AuthContext.refresh() probe can still surface the error
    // through their own catch handlers.
    if (res.status === 401 && !PUBLIC_AUTH_PATHS.has(url) && typeof window !== 'undefined') {
      // Avoid an infinite redirect loop if we're already on /login.
      if (window.location.pathname !== '/login') {
        window.location.assign('/login');
      }
    }
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
  isInitialized() {
    return request<{ initialized: boolean }>('/api/auth/initialized')
      .then((r) => r.initialized)
      .catch(() => false);
  },
  listUsers() {
    return request<UserSummary[]>('/api/auth/users');
  },
  createUser(input: { username: string; email: string; display_name: string; password: string; is_server_admin: boolean }) {
    return request<{ user: UserSummary }>('/api/auth/users', {
      method: 'POST',
      body: JSON.stringify(input),
    });
  },
  deleteUser(id: number) {
    return request<{ success: boolean }>(`/api/auth/users/${id}`, { method: 'DELETE' });
  },
  bootstrapAdmin(input: {
    username: string;
    email: string;
    display_name: string;
    password: string;
  }) {
    return request<{ user: User }>('/api/auth/bootstrap', {
      method: 'POST',
      body: JSON.stringify(input),
    });
  },

  // Personal Access Tokens
  listTokens() {
    return request<PatInfo[]>('/api/auth/tokens');
  },
  createToken(name: string, scopes: string[], expiresAt = 0) {
    return request<{ plaintext_token: string; pat: PatInfo }>('/api/auth/tokens', {
      method: 'POST',
      body: JSON.stringify({ name, scopes, expires_at: expiresAt }),
    });
  },
  deleteToken(id: number) {
    return request<{ success: boolean }>(`/api/auth/tokens/${id}`, { method: 'DELETE' });
  },

  // Sessions
  listSessions() {
    return request<SessionInfo[]>('/api/auth/sessions');
  },
  deleteSession(id: number) {
    return request<{ success: boolean }>(`/api/auth/sessions/${id}`, { method: 'DELETE' });
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
  createBranch(repo: string, name: string, baseBranch: string) {
    return request<{ success: boolean }>(`/api/repos/${enc(repo)}/branches`, {
      method: 'POST',
      body: JSON.stringify({ name, base_branch: baseBranch }),
    });
  },
  deleteBranch(repo: string, branch: string) {
    return request<{ success: boolean }>(`/api/repos/${enc(repo)}/branches/${enc(branch)}`, {
      method: 'DELETE',
    });
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
  acquireLock(repo: string, path: string, reason = '') {
    return request<{ granted: boolean }>(`/api/repos/${enc(repo)}/locks/acquire`, {
      method: 'POST',
      body: JSON.stringify({ path, owner: 'web-user', workspace_id: 'web', reason }),
    });
  },

  // Repo management
  updateRepo(repo: string, data: { new_name?: string; description?: string; visibility?: string; default_branch?: string }): Promise<{ success: boolean }> {
    return request(`/api/repos/${enc(repo)}`, { method: 'PUT', body: JSON.stringify(data) });
  },
  deleteRepo(repo: string): Promise<{ success: boolean }> {
    return request(`/api/repos/${enc(repo)}`, { method: 'DELETE' });
  },

  // Repo members (ACL)
  listRepoMembers(repo: string) {
    return request<RepoMember[]>(`/api/auth/repos/${enc(repo)}/members`);
  },
  addRepoMember(repo: string, userId: number, role: string) {
    return request<{ success: boolean }>(`/api/auth/repos/${enc(repo)}/members`, {
      method: 'POST',
      body: JSON.stringify({ user_id: userId, role }),
    });
  },
  removeRepoMember(repo: string, userId: number) {
    return request<{ success: boolean }>(`/api/auth/repos/${enc(repo)}/members/${userId}`, {
      method: 'DELETE',
    });
  },

  // User lookup (any authenticated user)
  lookupUser(username: string) {
    return request<UserSummary>(`/api/auth/users/lookup?username=${encodeURIComponent(username)}`);
  },

  // Comments
  listComments(repo: string, issueId: number, kind: 'issue' | 'pull_request' = 'issue') {
    return request<CommentInfo[]>(`/api/repos/${enc(repo)}/comments?issue_id=${issueId}&kind=${kind}`);
  },
  createComment(repo: string, issueId: number, body: string, kind: 'issue' | 'pull_request' = 'issue') {
    return request<{ success: boolean; id: number }>(`/api/repos/${enc(repo)}/comments`, {
      method: 'POST',
      body: JSON.stringify({ issue_id: issueId, kind, body }),
    });
  },
  updateComment(repo: string, commentId: number, body: string) {
    return request<{ success: boolean }>(`/api/repos/${enc(repo)}/comments/${commentId}`, {
      method: 'PUT',
      body: JSON.stringify({ body }),
    });
  },
  deleteComment(repo: string, commentId: number) {
    return request<{ success: boolean }>(`/api/repos/${enc(repo)}/comments/${commentId}`, {
      method: 'DELETE',
    });
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

/**
 * Build a navigation path segment for a `<owner>/<name>` repo identifier.
 *
 * Each segment is URL-encoded individually but the `/` between owner and
 * name is kept literal — that's what lets React Router split the path into
 * the `:owner/:repo` route params. Use this for `<Link to={...}>` and
 * `navigate(...)` targets, NOT for API call URLs (those need a single
 * encoded segment so axum's `:repo` param receives the full path after
 * per-segment decoding).
 */
export function repoPath(repo: string): string {
  return repo
    .split('/')
    .map(encodeURIComponent)
    .join('/');
}

import { createMockApi, mockData } from './mockApi';

export const splitRepo = (repo: string): [string, string] => {
  const idx = repo.indexOf('/');
  if (idx < 0) return ['', repo];
  return [repo.slice(0, idx), repo.slice(idx + 1)];
}

const isMock = import.meta.env.MODE === 'mock';

export default isMock ? createMockApi(api) : api;

export interface LanguageStat {
  name: string;
  color: string;
  percentage: number;
  bytes: number;
  count: number;
}

export async function getLanguageStats(repo: string, branch: string): Promise<LanguageStat[]> {
  if (isMock) {
    return [{ name: 'TypeScript', color: '#3178c6', percentage: 100, bytes: 1000, count: 10 }];
  }
  const resp = await fetch(`/api/repos/${encodeURIComponent(repo)}/stats/languages?branch=${encodeURIComponent(branch)}`);
  if (!resp.ok) return [];
  const data = await resp.json();
  return data.languages || [];
}

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
