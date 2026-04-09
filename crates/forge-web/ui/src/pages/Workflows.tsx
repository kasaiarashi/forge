import { useState, useEffect } from 'react';
import { useParams, Link, useNavigate } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import { Button, Flash, Spinner, TextInput, ActionMenu, ActionList, Label } from '@primer/react';
import {
  SearchIcon,
  CheckCircleIcon,
  XCircleFillIcon,
  DotFillIcon,
  SkipIcon,
  ClockIcon,
  WorkflowIcon,
  PlusIcon,
  PencilIcon,
  KebabHorizontalIcon,
} from '@primer/octicons-react';
import api from '../api';
import type { WorkflowInfo, RunInfo } from '../api';
import RepoHeader from '../components/RepoHeader';

function StatusIcon({ status, size = 16 }: { status: string; size?: number }) {
  switch (status) {
    case 'success':
      return <CheckCircleIcon size={size} className="status-icon-success" />;
    case 'failure':
      return <XCircleFillIcon size={size} className="status-icon-failure" />;
    case 'running':
      return <DotFillIcon size={size} className="status-icon-running" />;
    case 'cancelled':
      return <SkipIcon size={size} className="status-icon-cancelled" />;
    default:
      return <DotFillIcon size={size} className="status-icon-queued" />;
  }
}

function formatTimeWithTZ(ts: number): string {
  if (!ts) return '';
  return new Date(ts * 1000).toLocaleString('en-US', {
    month: 'short', day: 'numeric',
    hour: 'numeric', minute: '2-digit',
    timeZoneName: 'short',
  });
}

function duration(start: number, end: number): string {
  if (!start) return '';
  const d = (end || Math.floor(Date.now() / 1000)) - start;
  if (d < 60) return `${d}s`;
  return `${Math.floor(d / 60)}m ${d % 60}s`;
}

function extractBranch(ref: string): string {
  return ref.replace(/^refs\/heads\//, '');
}

export default function Workflows() {
  const repo = useRepoParam();
  const { id } = useParams<{ id?: string }>();
  const navigate = useNavigate();
  const selectedWorkflowId = id ? Number(id) : null;

  const [workflows, setWorkflows] = useState<WorkflowInfo[]>([]);
  const [runs, setRuns] = useState<RunInfo[]>([]);
  const [totalRuns, setTotalRuns] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [statusFilter, setStatusFilter] = useState<string>('');
  const [searchQuery, setSearchQuery] = useState('');

  const encRepo = encodeURIComponent(repo || '');

  const load = async () => {
    if (!repo) return;
    try {
      setLoading(true);
      const [wfs, runsRes] = await Promise.all([
        api.listWorkflows(repo),
        api.listRuns(repo, selectedWorkflowId || 0, 50, 0),
      ]);
      setWorkflows(wfs);
      setRuns(runsRes.runs);
      setTotalRuns(runsRes.total);
    } catch (e: any) {
      setError(e.message);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { load(); }, [repo, selectedWorkflowId]);

  const selectedWorkflow = workflows.find(w => w.id === selectedWorkflowId) || null;

  // Apply client-side filters
  let filteredRuns = runs;
  if (statusFilter) {
    filteredRuns = filteredRuns.filter(r => r.status === statusFilter);
  }
  if (searchQuery) {
    const q = searchQuery.toLowerCase();
    filteredRuns = filteredRuns.filter(r =>
      r.workflow_name.toLowerCase().includes(q) ||
      r.trigger.toLowerCase().includes(q) ||
      r.triggered_by.toLowerCase().includes(q)
    );
  }

  if (loading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
        <Spinner size="large" />
      </div>
    );
  }

  if (error) return <Flash variant="danger">{error}</Flash>;

  return (
    <div>
      <RepoHeader repo={repo || ''} currentTab="actions" />

      {/* Page header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4, marginTop: 16 }}>
        <h2 style={{ fontSize: '24px', fontWeight: 400, margin: 0 }}>Actions</h2>
        <Link to={`/${encRepo}/actions/new`} style={{ marginLeft: 'auto' }}>
          <Button variant="primary" size="small" leadingVisual={PlusIcon}>New workflow</Button>
        </Link>
      </div>
      <p style={{ color: 'var(--fg-muted)', fontSize: 13, margin: '0 0 16px 0' }}>
        Showing runs from {selectedWorkflow ? selectedWorkflow.name : 'all workflows'}
      </p>

      <div className="actions-layout">
        {/* ── LEFT SIDEBAR ── */}
        <nav className="actions-sidebar">
          <div
            className={`actions-nav-item ${!selectedWorkflowId ? 'active' : ''}`}
            onClick={() => navigate(`/${encRepo}/actions`)}
          >
            <WorkflowIcon size={16} />
            All workflows
          </div>

          {workflows.map(wf => (
            <div
              key={wf.id}
              className={`actions-nav-item ${selectedWorkflowId === wf.id ? 'active' : ''}`}
              onClick={() => navigate(`/${encRepo}/actions/${wf.id}/runs`)}
            >
              <WorkflowIcon size={16} />
              <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {wf.name}
              </span>
              {!wf.enabled && <Label size="small" variant="secondary">Off</Label>}
            </div>
          ))}

          <div className="sidebar-heading" style={{ marginTop: 16 }}>Management</div>
          <Link to={`/${encRepo}/actions/new`} className="actions-nav-item" style={{ color: 'var(--fg-default)' }}>
            <PlusIcon size={16} />
            New workflow
          </Link>
        </nav>

        {/* ── MAIN CONTENT ── */}
        <div className="actions-main">
          <div className="forge-card">
            {/* Filter bar */}
            <div className="filter-bar">
              <TextInput
                placeholder="Filter workflow runs"
                leadingVisual={SearchIcon}
                size="small"
                value={searchQuery}
                onChange={e => setSearchQuery(e.target.value)}
                style={{ flex: 1, minWidth: 200 }}
              />
              <ActionMenu>
                <ActionMenu.Button size="small">Status {statusFilter ? `· ${statusFilter}` : ''}</ActionMenu.Button>
                <ActionMenu.Overlay>
                  <ActionList selectionVariant="single">
                    <ActionList.Item selected={!statusFilter} onSelect={() => setStatusFilter('')}>
                      All
                    </ActionList.Item>
                    {['success', 'failure', 'running', 'queued', 'cancelled'].map(s => (
                      <ActionList.Item key={s} selected={statusFilter === s} onSelect={() => setStatusFilter(s)}>
                        <ActionList.LeadingVisual>
                          <StatusIcon status={s} size={14} />
                        </ActionList.LeadingVisual>
                        {s.charAt(0).toUpperCase() + s.slice(1)}
                      </ActionList.Item>
                    ))}
                  </ActionList>
                </ActionMenu.Overlay>
              </ActionMenu>
            </div>

            {/* Run count header */}
            <div style={{
              padding: '8px 16px',
              fontSize: 13,
              color: 'var(--fg-muted)',
              borderBottom: filteredRuns.length > 0 ? '1px solid var(--border-muted)' : 'none',
              fontWeight: 600,
            }}>
              {totalRuns} workflow run{totalRuns !== 1 ? 's' : ''}
            </div>

            {/* Run list */}
            {filteredRuns.length === 0 ? (
              <div style={{ padding: 48, textAlign: 'center', color: 'var(--fg-muted)' }}>
                {runs.length === 0
                  ? 'No runs yet. Trigger a workflow to get started.'
                  : 'No runs match the current filters.'}
              </div>
            ) : (
              filteredRuns.map(run => (
                <Link
                  key={run.id}
                  to={`/${encRepo}/actions/runs/${run.id}`}
                  className="run-row"
                >
                  {/* Status icon */}
                  <div style={{ paddingTop: 2 }}>
                    <StatusIcon status={run.status} size={18} />
                  </div>

                  {/* Title + subtitle */}
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{
                      fontWeight: 600,
                      fontSize: 14,
                      color: 'var(--fg-default)',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}>
                      {run.workflow_name}
                    </div>
                    <div style={{ fontSize: 12, color: 'var(--fg-muted)', marginTop: 2 }}>
                      {run.trigger} #{run.id}
                      {run.triggered_by && <> &middot; triggered by {run.triggered_by}</>}
                    </div>
                  </div>

                  {/* Right side: branch badge + timing */}
                  <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 4, flexShrink: 0 }}>
                    {run.trigger_ref && (
                      <span className="branch-badge">
                        {extractBranch(run.trigger_ref)}
                      </span>
                    )}
                    <div style={{ display: 'flex', alignItems: 'center', gap: 12, fontSize: 12, color: 'var(--fg-muted)' }}>
                      <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                        <ClockIcon size={12} />
                        {formatTimeWithTZ(run.created_at)}
                      </span>
                      {run.started_at > 0 && (
                        <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                          {duration(run.started_at, run.finished_at)}
                        </span>
                      )}
                    </div>
                  </div>

                  {/* Kebab menu */}
                  <div
                    onClick={e => e.preventDefault()}
                    style={{ flexShrink: 0, paddingTop: 2 }}
                  >
                    <ActionMenu>
                      <ActionMenu.Button
                        size="small"
                        variant="invisible"
                        aria-label="Run options"
                        icon={KebabHorizontalIcon}
                      />
                      <ActionMenu.Overlay>
                        <ActionList>
                          <ActionList.Item onSelect={() => navigate(`/${encRepo}/actions/runs/${run.id}`)}>
                            View run
                          </ActionList.Item>
                          {selectedWorkflow && (
                            <ActionList.Item onSelect={() => navigate(`/${encRepo}/actions/${selectedWorkflow.id}/edit`)}>
                              <ActionList.LeadingVisual><PencilIcon size={16} /></ActionList.LeadingVisual>
                              Edit workflow
                            </ActionList.Item>
                          )}
                        </ActionList>
                      </ActionMenu.Overlay>
                    </ActionMenu>
                  </div>
                </Link>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
