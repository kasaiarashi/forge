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
import PlaceholderPage from './pages/PlaceholderPage';

export default function App() {
  return (
    <Layout>
      <Routes>
        <Route path="/" element={<Dashboard />} />
        <Route path="/login" element={<Login />} />
        <Route path="/admin" element={<Admin />} />
        
        <Route path="/:repo" element={<RepoTree />} />
        <Route path="/:repo/tree/:branch" element={<RepoTree />} />
        <Route path="/:repo/tree/:branch/*" element={<RepoTree />} />
        <Route path="/:repo/blob/:branch/*" element={<FileView />} />
        <Route path="/:repo/commits/:branch" element={<Commits />} />
        <Route path="/:repo/commit/:hash" element={<CommitDetail />} />
        <Route path="/:repo/locks" element={<Locks />} />
        <Route path="/:repo/settings" element={<RepoSettings />} />
        
        <Route path="/:repo/actions" element={<Workflows />} />
        <Route path="/:repo/actions/new" element={<WorkflowEdit />} />
        <Route path="/:repo/actions/:id/edit" element={<WorkflowEdit />} />
        <Route path="/:repo/actions/:id/runs" element={<WorkflowRuns />} />
        <Route path="/:repo/actions/runs/:runId" element={<RunDetail />} />
        <Route path="/:repo/releases" element={<Releases />} />

        {/* New Dummy Tabs */}
        <Route path="/:repo/issues" element={<Issues />} />
        <Route path="/:repo/issues/new" element={<NewIssue />} />
        <Route path="/:repo/issues/:id" element={<IssueDetail />} />
        
        <Route path="/:repo/pulls" element={<PullRequests />} />
        <Route path="/:repo/pulls/new" element={<NewPullRequest />} />
        <Route path="/:repo/pulls/:id" element={<PullRequestDetail />} />
        <Route path="/:repo/projects" element={<PlaceholderPage tabName="projects" title="Welcome to Projects" description="Plan, track, and manage your work with projects." />} />
        <Route path="/:repo/security" element={<PlaceholderPage tabName="security" title="Security & Analysis" description="Understand the security of your code and help keep it safe." />} />
        <Route path="/:repo/insights" element={<PlaceholderPage tabName="insights" title="Repository Insights" description="Get data-driven insights into your community and development processes." />} />
      </Routes>
    </Layout>
  );
}
