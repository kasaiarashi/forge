import { useState, useEffect } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import { Button, Flash, Spinner, TextInput, Checkbox, FormControl } from '@primer/react';
import api, { repoPath } from '../api';

const DEFAULT_YAML = `name: My Workflow
on:
  push:
    branches: [main]
  manual: true

jobs:
  build:
    name: Build
    steps:
      - name: Hello
        run: echo "Hello from Forge Actions!"
`;

export default function WorkflowEdit() {
  const repo = useRepoParam();
  const { id } = useParams<{ id?: string }>();
  const navigate = useNavigate();
  const isNew = !id || id === 'new';

  const [name, setName] = useState('');
  const [yaml, setYaml] = useState(DEFAULT_YAML);
  const [enabled, setEnabled] = useState(true);
  const [loading, setLoading] = useState(!isNew);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!isNew && repo && id) {
      api.listWorkflows(repo).then(wfs => {
        const wf = wfs.find(w => w.id === Number(id));
        if (wf) {
          setName(wf.name);
          setYaml(wf.yaml);
          setEnabled(wf.enabled);
        }
        setLoading(false);
      }).catch(e => { setError(e.message); setLoading(false); });
    }
  }, [repo, id]);

  const handleSave = async () => {
    if (!repo) return;
    setSaving(true);
    setError('');
    try {
      if (isNew) {
        const res = await api.createWorkflow(repo, name, yaml);
        if (!res.success) throw new Error((res as any).error || 'Failed to create');
      } else {
        const res = await api.updateWorkflow(repo, Number(id), { name, yaml, enabled });
        if (!res.success) throw new Error((res as any).error || 'Failed to update');
      }
      navigate(`/${repoPath(repo)}/actions`);
    } catch (e: any) {
      setError(e.message);
    } finally {
      setSaving(false);
    }
  };

  if (loading) return <div style={{ padding: 32, textAlign: 'center' }}><Spinner /></div>;

  return (
    <div style={{ maxWidth: 960, margin: '0 auto', padding: '24px 16px' }}>
      <h2>{isNew ? 'New Workflow' : 'Edit Workflow'}</h2>
      {error && <Flash variant="danger" style={{ marginBottom: 16 }}>{error}</Flash>}

      <FormControl style={{ marginBottom: 16 }}>
        <FormControl.Label>Name</FormControl.Label>
        <TextInput value={name} onChange={e => setName(e.target.value)} placeholder="e.g. Build Game" block />
      </FormControl>

      {!isNew && (
        <FormControl style={{ marginBottom: 16 }}>
          <Checkbox checked={enabled} onChange={e => setEnabled((e.target as HTMLInputElement).checked)} />
          <FormControl.Label>Enabled</FormControl.Label>
        </FormControl>
      )}

      <FormControl style={{ marginBottom: 16 }}>
        <FormControl.Label>Workflow YAML</FormControl.Label>
        <textarea
          value={yaml}
          onChange={e => setYaml(e.target.value)}
          style={{
            width: '100%',
            minHeight: 400,
            fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
            fontSize: 13,
            padding: 12,
            border: '1px solid var(--border-default)',
            borderRadius: 6,
            backgroundColor: 'var(--bg-default)',
            color: 'var(--fg-default)',
            resize: 'vertical',
            tabSize: 2,
          }}
          spellCheck={false}
        />
      </FormControl>

      <div style={{ display: 'flex', gap: 8 }}>
        <Button variant="primary" onClick={handleSave} disabled={saving || !name.trim()}>
          {saving ? 'Saving...' : isNew ? 'Create Workflow' : 'Save Changes'}
        </Button>
        <Button onClick={() => navigate(`/${encodeURIComponent(repo!)}/actions`)}>Cancel</Button>
      </div>
    </div>
  );
}
