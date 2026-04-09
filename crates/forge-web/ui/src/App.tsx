import { Routes, Route } from 'react-router-dom';
import Layout from './components/Layout';
import Dashboard from './pages/Dashboard';
import Login from './pages/Login';
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


export default function App() {
  return (
    <Layout>
      <Routes>
        <Route path="/" element={<Dashboard />} />
        <Route path="/login" element={<Login />} />
        <Route path="/admin" element={<Admin />} />
        
        {/* User-namespaced repo routes: /<owner>/<repo>/... */}
        <Route path="/:owner/:repo" element={<RepoTree />} />
        <Route path="/:owner/:repo/tree/:branch" element={<RepoTree />} />
        <Route path="/:owner/:repo/tree/:branch/*" element={<RepoTree />} />
        <Route path="/:owner/:repo/blob/:branch/*" element={<FileView />} />
        <Route path="/:owner/:repo/commits/:branch" element={<Commits />} />
        <Route path="/:owner/:repo/commit/:hash" element={<CommitDetail />} />
        <Route path="/:owner/:repo/locks" element={<Locks />} />
        <Route path="/:owner/:repo/settings" element={<RepoSettings />} />

        <Route path="/:owner/:repo/actions" element={<Workflows />} />
        <Route path="/:owner/:repo/actions/new" element={<WorkflowEdit />} />
        <Route path="/:owner/:repo/actions/:id/edit" element={<WorkflowEdit />} />
        <Route path="/:owner/:repo/actions/:id/runs" element={<WorkflowRuns />} />
        <Route path="/:owner/:repo/actions/runs/:runId" element={<RunDetail />} />
        <Route path="/:owner/:repo/releases" element={<Releases />} />

        <Route path="/:owner/:repo/issues" element={<Issues />} />
        <Route path="/:owner/:repo/issues/new" element={<NewIssue />} />
        <Route path="/:owner/:repo/issues/:id" element={<IssueDetail />} />

        <Route path="/:owner/:repo/pulls" element={<PullRequests />} />
        <Route path="/:owner/:repo/pulls/new" element={<NewPullRequest />} />
        <Route path="/:owner/:repo/pulls/:id" element={<PullRequestDetail />} />
      </Routes>
    </Layout>
  );
}
