import { Routes, Route, Navigate, useLocation } from 'react-router-dom';
import { Spinner } from '@primer/react';
import Layout from './components/Layout';
import Dashboard from './pages/Dashboard';
import Login from './pages/Login';
import Setup from './pages/Setup';
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


function FullPageSpinner() {
  return (
    <div style={{ display: 'flex', justifyContent: 'center', padding: '64px 0' }}>
      <Spinner size="large" />
    </div>
  );
}

/**
 * Gate that fires before any other guard: if the server has zero users,
 * every route in the app gets redirected to /setup. Once the wizard
 * completes (or if the operator created the first admin via the CLI), this
 * guard becomes a no-op.
 */
function RequireSetup({ children }: { children: ReactNode }) {
  const { initialized, loading } = useAuth();
  if (loading || initialized === null) return <FullPageSpinner />;
  if (!initialized) return <Navigate to="/setup" replace />;
  return <>{children}</>;
}

/**
 * Redirect to /login if the user isn't authenticated. Used to wrap every
 * route that needs a session — keeps stale-cookie users from landing on a
 * page that flashes "401: invalid or expired session" before our request
 * helper can react. Always nest inside `RequireSetup` so an uninitialized
 * server bounces to the wizard before this even runs.
 */
function RequireAuth({ children }: { children: ReactNode }) {
  const { user, loading } = useAuth();
  const location = useLocation();
  if (loading) return <FullPageSpinner />;
  if (!user) {
    return <Navigate to="/login" replace state={{ from: location }} />;
  }
  return <>{children}</>;
}

/**
 * The /setup route uses the inverse gate: if the server is *already*
 * initialized, kick the visitor back to / instead of letting them re-run
 * the wizard (forge-server's BootstrapAdmin would reject the call anyway,
 * but we want a clean redirect, not an error).
 */
function OnlyWhenUninitialized({ children }: { children: ReactNode }) {
  const { initialized, loading } = useAuth();
  if (loading || initialized === null) return <FullPageSpinner />;
  if (initialized) return <Navigate to="/" replace />;
  return <>{children}</>;
}

/** Compose RequireSetup + RequireAuth for the common case. */
function Authenticated({ children }: { children: ReactNode }) {
  return (
    <RequireSetup>
      <RequireAuth>{children}</RequireAuth>
    </RequireSetup>
  );
}

export default function App() {
  return (
    <Layout>
      <Routes>
        {/* Setup wizard: only renders when no admin exists yet. Sits OUTSIDE
            both the auth and setup gates so it can be reached from a fresh
            install. */}
        <Route
          path="/setup"
          element={<OnlyWhenUninitialized><Setup /></OnlyWhenUninitialized>}
        />

        {/* Login: must wait for setup to complete first, then renders. */}
        <Route
          path="/login"
          element={<RequireSetup><Login /></RequireSetup>}
        />

        <Route path="/" element={<Authenticated><Dashboard /></Authenticated>} />
        <Route path="/admin" element={<Authenticated><Admin /></Authenticated>} />

        {/* User-namespaced repo routes: /<owner>/<repo>/... — all gated. */}
        <Route path="/:owner/:repo" element={<Authenticated><RepoTree /></Authenticated>} />
        <Route path="/:owner/:repo/tree/:branch" element={<Authenticated><RepoTree /></Authenticated>} />
        <Route path="/:owner/:repo/tree/:branch/*" element={<Authenticated><RepoTree /></Authenticated>} />
        <Route path="/:owner/:repo/blob/:branch/*" element={<Authenticated><FileView /></Authenticated>} />
        <Route path="/:owner/:repo/commits/:branch" element={<Authenticated><Commits /></Authenticated>} />
        <Route path="/:owner/:repo/commit/:hash" element={<Authenticated><CommitDetail /></Authenticated>} />
        <Route path="/:owner/:repo/locks" element={<Authenticated><Locks /></Authenticated>} />
        <Route path="/:owner/:repo/settings" element={<Authenticated><RepoSettings /></Authenticated>} />

        <Route path="/:owner/:repo/actions" element={<Authenticated><Workflows /></Authenticated>} />
        <Route path="/:owner/:repo/actions/new" element={<Authenticated><WorkflowEdit /></Authenticated>} />
        <Route path="/:owner/:repo/actions/:id/edit" element={<Authenticated><WorkflowEdit /></Authenticated>} />
        <Route path="/:owner/:repo/actions/:id/runs" element={<Authenticated><WorkflowRuns /></Authenticated>} />
        <Route path="/:owner/:repo/actions/runs/:runId" element={<Authenticated><RunDetail /></Authenticated>} />
        <Route path="/:owner/:repo/releases" element={<Authenticated><Releases /></Authenticated>} />

        <Route path="/:owner/:repo/issues" element={<Authenticated><Issues /></Authenticated>} />
        <Route path="/:owner/:repo/issues/new" element={<Authenticated><NewIssue /></Authenticated>} />
        <Route path="/:owner/:repo/issues/:id" element={<Authenticated><IssueDetail /></Authenticated>} />

        <Route path="/:owner/:repo/pulls" element={<Authenticated><PullRequests /></Authenticated>} />
        <Route path="/:owner/:repo/pulls/new" element={<Authenticated><NewPullRequest /></Authenticated>} />
        <Route path="/:owner/:repo/pulls/:id" element={<Authenticated><PullRequestDetail /></Authenticated>} />
      </Routes>
    </Layout>
  );
}
