import { useState, useEffect } from 'react';
import { useNavigate } from 'react-router-dom';
import { useRepoParam } from '../hooks/useRepoParam';
import { TextInput, Button, Flash, ActionMenu, ActionList, Spinner, Avatar } from '@primer/react';
import { GitPullRequestIcon, GitCommitIcon, CheckIcon, ArrowLeftIcon, MarkdownIcon, FileIcon, PersonIcon, GearIcon } from '@primer/octicons-react';
import RepoHeader from '../components/RepoHeader';
import api, { repoPath } from '../api';
import type { Branch, CommitSummary } from '../api';

export default function NewPullRequest() {
  const repo = useRepoParam();
  const navigate = useNavigate();

  const [branches, setBranches] = useState<Branch[]>([]);
  const [baseBranch, setBaseBranch] = useState('main');
  const [compareBranch, setCompareBranch] = useState('');
  
  // Create state variables
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [labels, setLabels] = useState('');
  const [commits, setCommits] = useState<CommitSummary[]>([]);
  
  // UI states
  const [loadingBranches, setLoadingBranches] = useState(true);
  const [error, setError] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [stage, setStage] = useState<'compare' | 'create'>('compare');
  const [writeMode, setWriteMode] = useState<'write' | 'preview'>('write');

  useEffect(() => {
    if (!repo) return;
    setLoadingBranches(true);
    api.listBranches(repo)
      .then(b => {
        setBranches(b);
        if (b.length > 0) {
          const main = b.find(br => br.name === 'main' || br.name === 'master');
          if (main) setBaseBranch(main.name);
          else setBaseBranch(b[0].name);
        }
      })
      .catch(e => setError(e.message))
      .finally(() => setLoadingBranches(false));
  }, [repo]);

  // Fetch commits when compare changes
  useEffect(() => {
    if (!repo || !compareBranch) {
      setCommits([]);
      return;
    }
    api.listCommits(repo, compareBranch, 1, 10)
      .then(res => setCommits(res.commits))
      .catch(() => setCommits([])); // Silently fail commits
  }, [repo, compareBranch]);

  const handleSubmit = async () => {
    if (!title.trim() || !compareBranch) return;
    setSubmitting(true);
    setError('');
    try {
      const labelList = labels.split(',').map(l => l.trim()).filter(Boolean);
      await api.createPullRequest(repo, title.trim(), compareBranch, baseBranch, body, labelList);
      navigate(`/${repoPath(repo)}/pulls`);
    } catch (e: any) {
      setError(e.message || 'Failed to create pull request');
      setSubmitting(false);
    }
  };

  if (loadingBranches) {
    return (
      <div>
        <RepoHeader repo={repo} currentTab="pulls" />
        <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
          <Spinner size="large" />
        </div>
      </div>
    );
  }

  const isValidComparison = baseBranch && compareBranch && baseBranch !== compareBranch;

  return (
    <div>
      <RepoHeader repo={repo} currentTab="pulls" />
      <div style={{ maxWidth: '1012px', margin: '0 auto', padding: '24px 16px', display: 'flex', flexDirection: 'column', gap: '24px' }}>
        
        {stage === 'compare' ? (
          <div>
            <h1 style={{ fontSize: '24px', fontWeight: 400, color: 'var(--fg-default)', margin: '0 0 8px 0' }}>Comparing changes</h1>
            <p style={{ color: 'var(--fg-muted)', fontSize: '14px', margin: '0 0 16px 0' }}>
              Choose two branches to see what's changed or to start a new pull request. If you need to, you can also compare across forks or learn more about diff comparisons.
            </p>
          </div>
        ) : (
          <div>
            <h1 style={{ fontSize: '24px', fontWeight: 400, color: 'var(--fg-default)', margin: '0 0 8px 0' }}>Open a pull request</h1>
            <p style={{ color: 'var(--fg-muted)', fontSize: '14px', margin: '0 0 16px 0' }}>
              Create a new pull request by comparing changes across two branches. If you need to, you can also compare across forks. <a href="#" style={{ color: 'var(--fg-accent)', textDecoration: 'none' }}>Learn more about diff comparisons here.</a>
            </p>
          </div>
        )}

        {error && <Flash variant="danger" style={{ marginBottom: 16 }}>{error}</Flash>}

        {/* Branch Selector Bar */}
        <div style={{ display: 'flex', alignItems: 'center', gap: '16px', padding: '16px', backgroundColor: 'var(--bg-subtle)', border: '1px solid var(--border-default)', borderRadius: '6px' }}>
          <span style={{ color: 'var(--fg-muted)' }}><GitPullRequestIcon /></span>
          
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <span style={{ color: 'var(--fg-muted)', fontSize: '14px' }}>base:</span>
            <ActionMenu>
              <ActionMenu.Button size="small" style={{ fontWeight: 600 }}>{baseBranch || 'Select branch'}</ActionMenu.Button>
              <ActionMenu.Overlay width="auto">
                <ActionList>
                  {branches.map(b => (
                    <ActionList.Item key={b.name} selected={baseBranch === b.name} onSelect={() => setBaseBranch(b.name)}>
                      {b.name}
                    </ActionList.Item>
                  ))}
                </ActionList>
              </ActionMenu.Overlay>
            </ActionMenu>
          </div>

          <span style={{ color: 'var(--fg-muted)' }}><ArrowLeftIcon /></span>

          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <span style={{ color: 'var(--fg-muted)', fontSize: '14px' }}>compare:</span>
            <ActionMenu>
              <ActionMenu.Button size="small" style={{ fontWeight: 600 }}>{compareBranch || 'compare'}</ActionMenu.Button>
              <ActionMenu.Overlay width="auto">
                <ActionList>
                  {branches.map(b => (
                    <ActionList.Item key={b.name} selected={compareBranch === b.name} onSelect={() => setCompareBranch(b.name)}>
                      {b.name}
                    </ActionList.Item>
                  ))}
                </ActionList>
              </ActionMenu.Overlay>
            </ActionMenu>
          </div>
          
          {isValidComparison && (
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px', color: 'var(--fg-success)', fontSize: '14px', marginLeft: '8px' }}>
              <CheckIcon />
              <strong>Able to merge.</strong> <span style={{ color: 'var(--fg-muted)' }}>These branches can be automatically merged.</span>
            </div>
          )}
        </div>

        {stage === 'compare' && isValidComparison && (
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '16px', backgroundColor: 'var(--bg-subtle)', border: '1px solid var(--border-default)', borderRadius: '6px' }}>
            <span style={{ color: 'var(--fg-default)', fontSize: '16px' }}>
              Discuss and review the changes in this comparison with others. <a href="#" style={{ color: 'var(--fg-accent)', textDecoration: 'none' }}>Learn about pull requests</a>
            </span>
            <Button variant="primary" onClick={() => { setStage('create'); setTitle(`Merge ${compareBranch} into ${baseBranch}`); }}>Create pull request</Button>
          </div>
        )}

        {stage === 'create' && (
          <div style={{ display: 'flex', gap: '24px' }}>
            <div style={{ flexShrink: 0 }}>
              <Avatar src="https://github.com/identicons/ghost.png" size={40} />
            </div>
            
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
                <div>
                  <label style={{ display: 'block', fontSize: '14px', fontWeight: 600, marginBottom: '8px' }}>Add a title *</label>
                  <TextInput
                    value={title}
                    onChange={(e) => setTitle(e.target.value)}
                    block
                    size="large"
                    style={{ backgroundColor: 'var(--bg-subtle)' }}
                  />
                </div>

                <div>
                  <label style={{ display: 'block', fontSize: '14px', fontWeight: 600, marginBottom: '8px' }}>Add a description</label>
                  <div style={{ border: '1px solid var(--border-default)', borderRadius: '6px', overflow: 'hidden' }}>
                    <div style={{ display: 'flex', backgroundColor: 'var(--bg-subtle)', borderBottom: '1px solid var(--border-default)' }}>
                      <button 
                        onClick={() => setWriteMode('write')}
                        style={{ padding: '8px 16px', background: writeMode === 'write' ? 'transparent' : 'var(--bg-subtle)', border: 'none', borderTop: writeMode === 'write' ? '2px solid var(--fg-accent)' : '2px solid transparent', borderRight: writeMode === 'write' ? '1px solid var(--border-default)' : 'none', borderLeft: writeMode === 'write' ? '1px solid var(--border-default)' : 'none', color: 'var(--fg-default)', fontSize: '14px', cursor: 'pointer', marginBottom: '-1px' }}>
                        Write
                      </button>
                      <button 
                        onClick={() => setWriteMode('preview')}
                        style={{ padding: '8px 16px', background: writeMode === 'preview' ? 'transparent' : 'var(--bg-subtle)', border: 'none', borderTop: writeMode === 'preview' ? '2px solid var(--fg-accent)' : '2px solid transparent', borderRight: writeMode === 'preview' ? '1px solid var(--border-default)' : 'none', borderLeft: writeMode === 'preview' ? '1px solid var(--border-default)' : 'none', color: 'var(--fg-default)', fontSize: '14px', cursor: 'pointer', marginBottom: '-1px' }}>
                        Preview
                      </button>
                    </div>
                    
                    {writeMode === 'write' && (
                      <div style={{ display: 'flex', padding: '8px', backgroundColor: 'var(--bg-subtle)', borderBottom: '1px solid var(--border-default)', gap: '12px', alignItems: 'center' }}>
                        <span style={{ color: 'var(--fg-muted)', fontWeight: 600, fontFamily: 'serif', padding: '0 4px' }}>B</span>
                        <span style={{ color: 'var(--fg-muted)', fontStyle: 'italic', fontFamily: 'serif', padding: '0 4px' }}>I</span>
                        <span style={{ color: 'var(--fg-muted)', padding: '0 4px', fontSize: '12px' }}>&lt;&gt;</span>
                        <div style={{ width: '1px', height: '16px', backgroundColor: 'var(--border-muted)' }}></div>
                        <span style={{ color: 'var(--fg-muted)' }}>@</span>
                      </div>
                    )}

                    {writeMode === 'write' ? (
                      <textarea
                        value={body}
                        onChange={(e) => setBody(e.target.value)}
                        placeholder="Add your description here..."
                        style={{
                          width: '100%',
                          minHeight: '400px',
                          padding: '12px',
                          border: 'none',
                          backgroundColor: 'transparent',
                          color: 'var(--fg-default)',
                          fontFamily: 'inherit',
                          resize: 'vertical',
                          outline: 'none'
                        }}
                      />
                    ) : (
                      <div style={{ minHeight: '400px', padding: '16px', color: 'var(--fg-default)', backgroundColor: 'transparent' }}>
                        {body || <span style={{ color: 'var(--fg-muted)' }}>Nothing to preview</span>}
                      </div>
                    )}
                    
                    <div style={{ display: 'flex', justifyContent: 'space-between', padding: '8px 12px', backgroundColor: 'var(--bg-subtle)', borderTop: '1px solid var(--border-default)', fontSize: '12px', color: 'var(--fg-muted)' }}>
                      <span style={{ display: 'flex', alignItems: 'center', gap: '4px' }}><MarkdownIcon /> Markdown is supported</span>
                      <span>Attach files by dragging & dropping, selecting or pasting them.</span>
                    </div>
                  </div>
                </div>

                <div style={{ display: 'flex', justifyContent: 'flex-end', paddingTop: '8px' }}>
                  <Button variant="primary" onClick={handleSubmit} disabled={submitting || !title.trim()}>
                    Create pull request
                  </Button>
                </div>
              </div>
            </div>

            <div style={{ width: '256px', flexShrink: 0 }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '16px', fontSize: '12px', color: 'var(--fg-muted)' }}>
                <div style={{ borderBottom: '1px solid var(--border-muted)', paddingBottom: '16px' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                    <span style={{ fontWeight: 600 }}>Reviewers</span>
                    <GearIcon />
                  </div>
                  <span>No reviews—<a href="#" style={{ color: 'var(--fg-muted)', textDecoration: 'none' }}>request one</a></span>
                </div>
                
                <div style={{ borderBottom: '1px solid var(--border-muted)', paddingBottom: '16px' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                    <span style={{ fontWeight: 600 }}>Assignees</span>
                    <GearIcon />
                  </div>
                  <span>No one—<a href="#" style={{ color: 'var(--fg-muted)', textDecoration: 'none' }}>assign yourself</a></span>
                </div>

                <div style={{ borderBottom: '1px solid var(--border-muted)', paddingBottom: '16px' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                    <span style={{ fontWeight: 600 }}>Labels</span>
                    <GearIcon />
                  </div>
                  <input
                    type="text"
                    value={labels}
                    onChange={e => setLabels(e.target.value)}
                    placeholder="bug, feature, ..."
                    style={{ width: '100%', padding: '4px 8px', fontSize: '12px', borderRadius: '4px', border: '1px solid var(--border-default)', backgroundColor: 'var(--bg-default)', color: 'var(--fg-default)', boxSizing: 'border-box' }}
                  />
                </div>

                <div style={{ borderBottom: '1px solid var(--border-muted)', paddingBottom: '16px' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                    <span style={{ fontWeight: 600 }}>Projects</span>
                    <GearIcon />
                  </div>
                  <span>None yet</span>
                </div>

                <div style={{ borderBottom: '1px solid var(--border-muted)', paddingBottom: '16px' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '8px' }}>
                    <span style={{ fontWeight: 600 }}>Milestone</span>
                    <GearIcon />
                  </div>
                  <span>No milestone</span>
                </div>
              </div>
            </div>
          </div>
        )}

        {/* Commits visual mock up */}
        {isValidComparison && commits.length > 0 && (
          <div style={{ marginTop: '24px' }}>
            <div style={{ display: 'flex', alignItems: 'center', border: '1px solid var(--border-default)', borderRadius: '6px', padding: '8px', fontSize: '14px', color: 'var(--fg-muted)' }}>
              <div style={{ flex: 1, textAlign: 'center', borderRight: '1px solid var(--border-muted)' }}>
                <GitCommitIcon /> <strong>{commits.length}</strong> commit{commits.length !== 1 ? 's' : ''}
              </div>
              <div style={{ flex: 1, textAlign: 'center', borderRight: '1px solid var(--border-muted)' }}>
                <FileIcon /> <strong>1</strong> file changed
              </div>
              <div style={{ flex: 1, textAlign: 'center' }}>
                <PersonIcon /> <strong>1</strong> contributor
              </div>
            </div>

            <div style={{ marginLeft: '16px', borderLeft: '2px solid var(--border-muted)', paddingLeft: '16px', paddingTop: '24px' }}>
              <div style={{ fontSize: '12px', color: 'var(--fg-muted)', display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
                <GitCommitIcon /> Commits on {new Date(commits[0].timestamp * 1000).toLocaleDateString()}
              </div>

              {commits.map(commit => (
                <div key={commit.hash} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '12px 16px', border: '1px solid var(--border-default)', borderRadius: '6px', marginBottom: '8px', backgroundColor: 'var(--bg-default)' }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                    <Avatar src="https://github.com/identicons/ghost.png" size={20} />
                    <span style={{ fontWeight: 600, fontSize: '14px', color: 'var(--fg-default)' }}>{commit.message.split('\\n')[0]}</span>
                    <span style={{ fontSize: '12px', color: 'var(--fg-muted)' }}>
                      {commit.author_name} committed {new Date(commit.timestamp * 1000).toLocaleDateString()}
                    </span>
                  </div>
                  <div style={{ display: 'flex', gap: '8px' }}>
                    <code style={{ fontSize: '12px', color: 'var(--fg-default)' }}>{commit.hash.substring(0, 7)}</code>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

      </div>
    </div>
  );
}
