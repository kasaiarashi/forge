import { useState } from 'react';
import { useParams, useNavigate, Link } from 'react-router-dom';
import { TextInput, Button, FormControl, Flash } from '@primer/react';
import RepoHeader from '../components/RepoHeader';
import api from '../api';

export default function NewIssue() {
  const { repo = '' } = useParams();
  const navigate = useNavigate();
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [labels, setLabels] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState('');

  const handleSubmit = async () => {
    if (!title.trim()) return;
    setSubmitting(true);
    setError('');
    try {
      const labelArray = labels
        .split(',')
        .map(l => l.trim())
        .filter(l => l.length > 0);
      await api.createIssue(repo, title.trim(), body, labelArray);
      navigate(`/${encodeURIComponent(repo)}/issues`);
    } catch (e: any) {
      setError(e.message || 'Failed to create issue');
      setSubmitting(false);
    }
  };

  return (
    <div>
      <RepoHeader repo={repo} currentTab="issues" />
      <div style={{ maxWidth: '1012px', margin: '0 auto', padding: '0 16px', display: 'flex', gap: '24px' }}>
        
        {/* Main form */}
        <div style={{ flex: 1 }}>
          {error && <Flash variant="danger" style={{ marginBottom: 16 }}>{error}</Flash>}
          
          <div className="forge-card" style={{ padding: '24px', border: '1px solid var(--border-default)', borderRadius: '6px' }}>
            <FormControl>
              <FormControl.Label style={{ fontSize: '16px', marginBottom: '8px' }}>Add a title</FormControl.Label>
              <TextInput
                autoFocus
                size="large"
                block
                placeholder="Title"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                style={{ backgroundColor: 'var(--bg-subtle)' }}
              />
            </FormControl>

            <FormControl style={{ marginTop: '16px' }}>
              <FormControl.Label style={{ fontSize: '14px', marginBottom: '8px' }}>Add a description</FormControl.Label>
              <textarea
                className="form-control"
                placeholder="Leave a comment"
                value={body}
                onChange={(e) => setBody(e.target.value)}
                style={{
                  width: '100%',
                  minHeight: '200px',
                  padding: '12px',
                  borderRadius: '6px',
                  border: '1px solid var(--border-default)',
                  backgroundColor: 'var(--bg-subtle)',
                  color: 'var(--fg-default)',
                  fontFamily: 'inherit',
                  resize: 'vertical'
                }}
              />
            </FormControl>

            <div style={{ display: 'flex', justifyContent: 'flex-start', alignItems: 'center', gap: '16px', marginTop: '24px' }}>
              <Button variant="primary" onClick={handleSubmit} disabled={submitting || !title.trim()}>
                Submit new issue
              </Button>
              <Link to={`/${encodeURIComponent(repo)}/issues`} style={{ color: 'var(--fg-muted)', textDecoration: 'none' }}>
                Cancel
              </Link>
            </div>
          </div>
        </div>

        {/* Sidebar */}
        <div style={{ width: '256px', flexShrink: 0 }}>
          <div style={{ borderBottom: '1px solid var(--border-muted)', paddingBottom: '16px' }}>
            <h3 style={{ fontSize: '12px', fontWeight: 600, color: 'var(--fg-muted)', marginBottom: '8px', textTransform: 'uppercase' }}>Labels</h3>
            <TextInput
              block
              placeholder="e.g. bug, help wanted"
              value={labels}
              onChange={(e) => setLabels(e.target.value)}
              style={{ backgroundColor: 'var(--bg-subtle)', fontSize: '12px' }}
            />
            <p style={{ fontSize: '12px', color: 'var(--fg-muted)', margin: '8px 0 0 0' }}>Comma-separated</p>
          </div>
        </div>
        
      </div>
    </div>
  );
}
