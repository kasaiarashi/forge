import { Routes, Route, Navigate, useLocation } from 'react-router-dom';
import { Spinner } from '@primer/react';
import Layout from './components/Layout';
import Dashboard from './pages/Dashboard';
import Login from './pages/Login';
import { useAuth } from './context/AuthContext';
import type { ReactNode } from 'react';
import RepoTree from './pages/RepoTree';
import FileView from './pages/FileView';
import Commits from './pages/Commits';
import CommitDetail from './pages/CommitDetail';
import Locks from './pages/Locks';
import RepoSettings from './pages/RepoSettings';
import Admin from './pages/Admin';
import Workflows from './pages/Workflows';
import WorkflowEdit from './pages/WorkflowEdit';
import WorkflowRuns from './pages/WorkflowRuns';
import RunDetail from './pages/RunDetail';
import Releases from './pages/Releases';
import Issues from './pages/Issues';
import NewIssue from './pages/NewIssue';
import IssueDetail from './pages/IssueDetail';
import PullRequests from './pages/PullRequests';
import NewPullRequest from './pages/NewPullRequest';
import PullRequestDetail from './pages/PullRequestDetail';


/// Redirect to /login if the user isn't authenticated. Used to wrap every
/// route that needs a session — keeps stale-cookie users from landing on a
/// page that flashes "401: invalid or expired session" before our request
/// helper can react.
function RequireAuth({ children }: { children: ReactNode }) {
  const { user, loading } = useAuth();
  const location = useLocation();
  if (loading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: '64px 0' }}>
        <Spinner size="large" />
      </div>
    );
  }
  if (!user) {
    return <Navigate to="/login" replace state={{ from: location }} />;
  }
  return <>{children}</>;
}

export default function App() {
  return (
    <Layout>
      <Routes>
        <Route path="/login" element={<Login />} />
        <Route path="/" element={<RequireAuth><Dashboard /></RequireAuth>} />
        <Route path="/admin" element={<RequireAuth><Admin /></RequireAuth>} />
        
        {/* User-namespaced repo routes: /<owner>/<repo>/... — all gated by RequireAuth */}
        <Route path="/:owner/:repo" element={<RequireAuth><RepoTree /></RequireAuth>} />
        <Route path="/:owner/:repo/tree/:branch" element={<RequireAuth><RepoTree /></RequireAuth>} />
        <Route path="/:owner/:repo/tree/:branch/*" element={<RequireAuth><RepoTree /></RequireAuth>} />
        <Route path="/:owner/:repo/blob/:branch/*" element={<RequireAuth><FileView /></RequireAuth>} />
        <Route path="/:owner/:repo/commits/:branch" element={<RequireAuth><Commits /></RequireAuth>} />
        <Route path="/:owner/:repo/commit/:hash" element={<RequireAuth><CommitDetail /></RequireAuth>} />
        <Route path="/:owner/:repo/locks" element={<RequireAuth><Locks /></RequireAuth>} />
        <Route path="/:owner/:repo/settings" element={<RequireAuth><RepoSettings /></RequireAuth>} />

        <Route path="/:owner/:repo/actions" element={<RequireAuth><Workflows /></RequireAuth>} />
        <Route path="/:owner/:repo/actions/new" element={<RequireAuth><WorkflowEdit /></RequireAuth>} />
        <Route path="/:owner/:repo/actions/:id/edit" element={<RequireAuth><WorkflowEdit /></RequireAuth>} />
        <Route path="/:owner/:repo/actions/:id/runs" element={<RequireAuth><WorkflowRuns /></RequireAuth>} />
        <Route path="/:owner/:repo/actions/runs/:runId" element={<RequireAuth><RunDetail /></RequireAuth>} />
        <Route path="/:owner/:repo/releases" element={<RequireAuth><Releases /></RequireAuth>} />

        <Route path="/:owner/:repo/issues" element={<RequireAuth><Issues /></RequireAuth>} />
        <Route path="/:owner/:repo/issues/new" element={<RequireAuth><NewIssue /></RequireAuth>} />
        <Route path="/:owner/:repo/issues/:id" element={<RequireAuth><IssueDetail /></RequireAuth>} />

        <Route path="/:owner/:repo/pulls" element={<RequireAuth><PullRequests /></RequireAuth>} />
        <Route path="/:owner/:repo/pulls/new" element={<RequireAuth><NewPullRequest /></RequireAuth>} />
        <Route path="/:owner/:repo/pulls/:id" element={<RequireAuth><PullRequestDetail /></RequireAuth>} />
      </Routes>
    </Layout>
  );
}
