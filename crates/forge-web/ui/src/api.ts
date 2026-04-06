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
}

export interface TreeResponse {
  commit_hash: string;
  path: string;
  entries: TreeEntry[];
}

export interface FileContent {
  content: string | null;
  size: number;
  is_binary: boolean;
  hash: string;
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
};

function enc(s: string) {
  return encodeURIComponent(s);
}

export default api;
