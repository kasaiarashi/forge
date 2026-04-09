import { useState, useEffect } from 'react';
import { useParams, Link } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import { Button, Flash, Spinner, Label } from '@primer/react';
import {
  CheckCircleIcon,
  XCircleFillIcon,
  XCircleIcon,
  DotFillIcon,
  SkipIcon,
  ArrowLeftIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  PackageIcon,
  ClockIcon,
  PlayIcon,
  FileIcon,
} from '@primer/octicons-react';
import api from '../api';
import type { RunDetail as RunDetailData } from '../api';
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

function StatusBadge({ status }: { status: string }) {
  const color: Record<string, string> = {
    success: 'var(--fg-success)',
    failure: 'var(--fg-danger)',
    running: 'var(--fg-warning)',
    queued: 'var(--fg-muted)',
    cancelled: 'var(--fg-muted)',
  };
  return (
    <span style={{
      fontWeight: 600,
      color: color[status] || 'var(--fg-muted)',
      textTransform: 'capitalize',
    }}>
      {status}
    </span>
  );
}

function formatTime(ts: number): string {
  if (!ts) return '-';
  return new Date(ts * 1000).toLocaleString();
}

function duration(start: number, end: number): string {
  if (!start) return '-';
  const d = (end || Math.floor(Date.now() / 1000)) - start;
  if (d < 60) return `${d}s`;
  return `${Math.floor(d / 60)}m ${d % 60}s`;
}

function stepDuration(start: number, end: number): string {
  if (!start || !end) return '';
  const d = end - start;
  if (d < 60) return `${d}s`;
  return `${Math.floor(d / 60)}m ${d % 60}s`;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function deriveJobStatus(steps: { status: string }[]): string {
  if (steps.some(s => s.status === 'failure')) return 'failure';
  if (steps.some(s => s.status === 'running')) return 'running';
  if (steps.some(s => s.status === 'queued')) return 'queued';
  if (steps.some(s => s.status === 'cancelled')) return 'cancelled';
  if (steps.every(s => s.status === 'success')) return 'success';
  return 'queued';
}

export default function RunDetail() {
  const repo = useRepoParam();
  const { runId } = useParams<{ runId: string }>();
  const [data, setData] = useState<RunDetailData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [expandedSteps, setExpandedSteps] = useState<Set<number>>(new Set());

  useEffect(() => {
    if (!repo || !runId) return;
    api.getRun(repo, Number(runId))
      .then(setData)
      .catch(e => setError(e.message))
      .finally(() => setLoading(false));
  }, [repo, runId]);

  const toggleStep = (id: number) => {
    setExpandedSteps(prev => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  };

  const handleCancel = async () => {
    if (!repo || !runId) return;
    await api.cancelRun(repo, Number(runId));
    const updated = await api.getRun(repo, Number(runId));
    setData(updated);
  };

  const handleRerun = async () => {
    if (!repo || !data?.run) return;
    await api.triggerWorkflow(repo, data.run.workflow_id);
  };

  if (loading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
        <Spinner size="large" />
      </div>
    );
  }

  if (error) return <Flash variant="danger">{error}</Flash>;
  if (!data?.run) return <Flash variant="danger">Run not found</Flash>;

  const { run, steps, artifacts } = data;
  const encRepo = encodeURIComponent(repo!);

  // Group steps by job_name
  const jobNames = [...new Set(steps.map(s => s.job_name))];
  const jobGroups = jobNames.map(name => ({
    name,
    steps: steps.filter(s => s.job_name === name),
    status: deriveJobStatus(steps.filter(s => s.job_name === name)),
  }));

  return (
    <div>
      <RepoHeader repo={repo!} currentTab="actions" />

      {/* Back breadcrumb */}
      <Link
        to={`/${encRepo}/actions/${run.workflow_id}/runs`}
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
          fontSize: 13,
          color: 'var(--fg-accent)',
          textDecoration: 'none',
          marginBottom: 12,
        }}
      >
        <ArrowLeftIcon size={16} />
        {run.workflow_name}
      </Link>

      {/* Header: status icon + title + run number */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 16 }}>
        <StatusIcon status={run.status} size={24} />
        <h2 style={{ margin: 0, fontWeight: 400, fontSize: 24 }}>
          {run.workflow_name}
          <span style={{ color: 'var(--fg-muted)', fontWeight: 400 }}> #{run.id}</span>
        </h2>
        <div style={{ flex: 1 }} />
        {(run.status === 'queued' || run.status === 'running') && (
          <Button variant="danger" leadingVisual={XCircleIcon} onClick={handleCancel}>
            Cancel run
          </Button>
        )}
        <Button leadingVisual={PlayIcon} onClick={handleRerun}>
          Re-run all jobs
        </Button>
      </div>

      {/* Info bar */}
      <div className="run-info-bar">
        <div>
          <span>Triggered via</span>
          <span>{run.trigger} {run.triggered_by && `by ${run.triggered_by}`}</span>
        </div>
        <div>
          <span>Status</span>
          <StatusBadge status={run.status} />
        </div>
        <div>
          <span>Total duration</span>
          <span>{duration(run.started_at, run.finished_at)}</span>
        </div>
        <div>
          <span>Artifacts</span>
          <span>{artifacts.length > 0 ? artifacts.length : '–'}</span>
        </div>
      </div>

      {/* Two-panel layout */}
      <div className="actions-layout">
        {/* ── LEFT SIDEBAR: Jobs ── */}
        <nav className="actions-sidebar">
          <div className="forge-card" style={{ padding: '8px 0' }}>
            {/* Summary */}
            <div className="job-list-item active" style={{ fontWeight: 600 }}>
              <ClockIcon size={16} />
              Summary
            </div>

            {/* All jobs header */}
            <div className="sidebar-heading">All jobs</div>

            {jobGroups.map(job => (
              <a
                key={job.name}
                href={`#job-${job.name}`}
                className="job-list-item"
                onClick={e => {
                  e.preventDefault();
                  document.getElementById(`job-${job.name}`)?.scrollIntoView({ behavior: 'smooth' });
                }}
              >
                <StatusIcon status={job.status} size={16} />
                <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                  {job.name}
                </span>
              </a>
            ))}

            {/* Run details */}
            <div className="sidebar-heading" style={{ marginTop: 8 }}>Run details</div>
            <div className="job-list-item" style={{ color: 'var(--fg-muted)', fontSize: 12, cursor: 'default' }}>
              <ClockIcon size={14} />
              {formatTime(run.created_at)}
            </div>
            {run.commit_hash && (
              <div className="job-list-item" style={{ color: 'var(--fg-muted)', fontSize: 12, cursor: 'default' }}>
                <FileIcon size={14} />
                <code style={{ fontSize: 12 }}>{run.commit_hash.slice(0, 12)}</code>
              </div>
            )}
          </div>
        </nav>

        {/* ── MAIN CONTENT ── */}
        <div className="actions-main">
          {/* Job cards */}
          {jobGroups.map(job => (
            <div key={job.name} id={`job-${job.name}`} className="forge-card" style={{ marginBottom: 16 }}>
              <div className="forge-card-header" style={{ gap: 8 }}>
                <StatusIcon status={job.status} size={16} />
                <strong style={{ fontSize: 14 }}>{job.name}</strong>
                <span style={{ fontSize: 12, color: 'var(--fg-muted)', marginLeft: 'auto' }}>
                  on: {run.trigger}
                </span>
              </div>

              {/* Workflow visualization card */}
              <div style={{
                margin: 16,
                border: '1px solid var(--border-default)',
                borderRadius: 6,
                overflow: 'hidden',
              }}>
                {job.steps.map(step => (
                  <div key={step.id}>
                    <div
                      className="step-row"
                      onClick={() => toggleStep(step.id)}
                    >
                      <span style={{ color: 'var(--fg-muted)', flexShrink: 0, display: 'inline-flex' }}>
                        {expandedSteps.has(step.id)
                          ? <ChevronDownIcon size={16} />
                          : <ChevronRightIcon size={16} />
                        }
                      </span>
                      <StatusIcon status={step.status} size={16} />
                      <span style={{ fontWeight: 500, flex: 1 }}>{step.name}</span>
                      {step.started_at > 0 && step.finished_at > 0 && (
                        <span style={{ fontSize: 12, color: 'var(--fg-muted)', flexShrink: 0 }}>
                          {stepDuration(step.started_at, step.finished_at)}
                        </span>
                      )}
                    </div>

                    {expandedSteps.has(step.id) && step.log && (
                      <pre className="step-log">{step.log}</pre>
                    )}
                  </div>
                ))}
              </div>
            </div>
          ))}

          {/* No steps */}
          {jobGroups.length === 0 && (
            <div className="forge-card" style={{ padding: 32, textAlign: 'center', color: 'var(--fg-muted)' }}>
              No steps recorded for this run.
            </div>
          )}

          {/* Artifacts */}
          {artifacts.length > 0 && (
            <div className="forge-card" style={{ marginBottom: 16 }}>
              <div className="forge-card-header" style={{ gap: 8 }}>
                <PackageIcon size={16} />
                <strong>Artifacts</strong>
                <Label size="small" variant="secondary">{artifacts.length}</Label>
              </div>
              {artifacts.map((a, i) => (
                <div key={a.id} style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  padding: '8px 16px',
                  borderBottom: i < artifacts.length - 1 ? '1px solid var(--border-muted)' : 'none',
                  fontSize: 13,
                }}>
                  <span style={{ fontWeight: 500 }}>{a.name}</span>
                  <span style={{ color: 'var(--fg-muted)' }}>{formatSize(a.size_bytes)}</span>
                </div>
              ))}
            </div>
          )}

          {/* Annotations section */}
          {steps.some(s => s.exit_code > 0) && (
            <div className="forge-card">
              <div className="forge-card-header" style={{ gap: 8 }}>
                <strong>Annotations</strong>
                <Label size="small" variant="secondary">
                  {steps.filter(s => s.exit_code > 0).length}
                </Label>
              </div>
              {steps.filter(s => s.exit_code > 0).map(step => (
                <div key={step.id} style={{
                  padding: '12px 16px',
                  borderBottom: '1px solid var(--border-muted)',
                  fontSize: 13,
                }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
                    <XCircleFillIcon size={16} className="status-icon-failure" />
                    <strong>{step.name}</strong>
                  </div>
                  <div style={{ color: 'var(--fg-muted)', fontSize: 12 }}>
                    Process exited with code {step.exit_code} in job {step.job_name}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
