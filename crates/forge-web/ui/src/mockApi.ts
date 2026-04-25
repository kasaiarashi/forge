import type { 
  User, RepoInfo, Branch, CommitList, TreeResponse, FileContent, 
  PullRequestListResponse, IssueListResponse, WorkflowInfo, RunInfo
} from './api';

const MOCK_DELAY = 300;

function delay<T>(data: T): Promise<T> {
  return new Promise(resolve => setTimeout(() => resolve(data), MOCK_DELAY));
}

export const mockData = {
  user: {
    username: 'mockuser',
    is_admin: true,
  } as User,

  repos: [
    { name: 'mockuser/forge-demo', description: 'A mock repository for UI testing', is_public: true, default_branch: 'main', visibility: 'public', updated_at: Date.now() / 1000 },
    { name: 'mockuser/empty-repo', description: 'An empty mock repository', is_public: false, default_branch: 'main', visibility: 'private', updated_at: Date.now() / 1000 }
  ] as RepoInfo[],

  branches: [
    { name: 'main', head: 'abc1234567890def' },
    { name: 'feature/mock-ui', head: 'def0987654321abc' }
  ] as Branch[],

  commits: {
    commits: Array.from({ length: 30 }).map((_, i) => ({
      hash: `mockhash${i}${Date.now()}`,
      message: `Mock commit message ${i}`,
      author_name: 'Mock User',
      author_email: 'mock@example.com',
      timestamp: Math.floor(Date.now() / 1000) - (i * 3600),
      parent_hashes: []
    })),
    total: 100
  } as CommitList,

  tree: {
    entries: [
      { name: 'src', kind: 'directory', size: 0, hash: 'dirhash' },
      { name: 'public', kind: 'directory', size: 0, hash: 'dirhash' },
      { name: 'package.json', kind: 'file', size: 1024, hash: 'filehash' },
      { name: 'README.md', kind: 'file', size: 2048, hash: 'filehash' }
    ]
  } as TreeResponse,

  fileContent: {
    content: btoa('# Mock File Content\n\nThis is a mocked file returned by the mock API client.'),
    is_binary: false,
    size: 60
  } as FileContent,
};

// Generic proxy generator for API
export const createMockApi = (realApi: any) => {
  return new Proxy(realApi, {
    get(target, prop: string) {
      // Intercept purely specific data fetching paths
      if (prop === 'me') return () => delay(mockData.user);
      if (prop === 'isInitialized') return () => delay(true);
      if (prop === 'listRepos') return () => delay(mockData.repos);
      if (prop === 'listBranches') return (repo: string) => delay(repo.includes('empty') ? [] : mockData.branches);
      if (prop === 'listCommits') return () => delay(mockData.commits);
      if (prop === 'getTree') return () => delay(mockData.tree);
      if (prop === 'getFile') return () => delay(mockData.fileContent);
      if (prop === 'getLanguageStats') return () => delay([{ name: 'TypeScript', color: '#3178c6', percentage: 100, bytes: 1000, count: 10 }]);
      if (prop === 'listPullRequests') return () => delay({ pull_requests: [], total: 0 } as PullRequestListResponse);
      if (prop === 'listIssues') return () => delay({ issues: [], total: 0 } as IssueListResponse);
      if (prop === 'listWorkflows') return () => delay([] as WorkflowInfo[]);
      if (prop === 'listRuns') return () => delay({ runs: [], total: 0 } as unknown as { runs: RunInfo[], total: number });
      if (prop === 'createBranch') return async (repo: string, name: string, baseBranch: string) => {
        mockData.branches.push({ name, head: 'mockhead' });
        return delay({ success: true });
      };
      if (prop === 'deleteBranch') return async (repo: string, branch: string) => {
        mockData.branches = mockData.branches.filter(b => b.name !== branch);
        return delay({ success: true });
      };

      // For everything else, log and return empty successes
      return async (...args: any[]) => {
        console.warn(`[Mock API] Called unimplemented mock for ${prop}`, args);
        return { success: true };
      };
    }
  });
};
