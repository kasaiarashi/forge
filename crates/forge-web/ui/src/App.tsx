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
      </Routes>
    </Layout>
  );
}
