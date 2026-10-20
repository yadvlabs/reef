import { Button, RelativeTime } from '@primer/react';
import { GitBranchIcon, StopwatchIcon, SyncIcon, XIcon, ChevronDownIcon, ChevronRightIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { useState } from 'react';
import { StatusIcon, statusLabel } from '../components/ActionsBits';
import { ErrorFlash, Loading } from '../components/Loading';
import { api } from '../lib/api';
import { jobLogQuery, runDuration, runJobsQuery, runQuery, splitLog, type Job } from '../lib/actions';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/actions/runs/$id')({
  validateSearch: (s: Record<string, unknown>): { job?: number } => (typeof s.job === 'number' ? { job: s.job } : {}),
  component: RunPage,
});

function RunPage() {
  const repo = useRepo();
  const { id } = Route.useParams();
  const { job: jobParam } = Route.useSearch();
  const qc = useQueryClient();
  const run = useQuery(runQuery(repo.full_name, Number(id)));
  const jobs = useQuery(runJobsQuery(repo.full_name, Number(id)));
  const p = { owner: repo.owner.login, repo: repo.name };
  const invalidate = () => qc.invalidateQueries({ queryKey: ['actions', repo.full_name.toLowerCase()] });
  const cancel = useMutation({ mutationFn: () => api(`/repos/${repo.full_name}/actions/runs/${id}/cancel`, { method: 'POST' }), onSuccess: invalidate });
  const rerun = useMutation({ mutationFn: (failed: boolean) => api(`/repos/${repo.full_name}/actions/runs/${id}/${failed ? 'rerun-failed-jobs' : 'rerun'}`, { method: 'POST' }), onSuccess: invalidate });
  if (run.isPending) return <Loading />;
  if (run.isError) return <ErrorFlash error={run.error} />;
  const r = run.data;
  const list = jobs.data?.jobs ?? [];
  const selected = list.find((j) => j.id === jobParam) ?? list.find((j) => j.status === 'in_progress') ?? list.find((j) => j.conclusion === 'failure') ?? list[0];
  return (
    <div>
      <div className="run-head">
        <StatusIcon status={r.status} conclusion={r.conclusion} size={24} />
        <div style={{ flex: 1, minWidth: 0 }}>
          <h1>
            {r.display_title} <span className="muted" style={{ fontWeight: 400 }}>{r.name} #{r.run_number}</span>
          </h1>
          <div className="muted small" style={{ display: 'flex', gap: 12, flexWrap: 'wrap' }}>
            <span>{statusLabel(r.status, r.conclusion)}</span>
            {r.head_branch ? (
              <span className="branch-pill">
                <GitBranchIcon /> {r.head_branch}
              </span>
            ) : null}
            <Link to="/$owner/$repo/commit/$sha" params={{ ...p, sha: r.head_sha }}>
              <code>{r.head_sha.slice(0, 7)}</code>
            </Link>
            <span>{r.event}</span>
            <span>
              <RelativeTime datetime={r.created_at} />
            </span>
            <span>
              <StopwatchIcon /> {runDuration(r)}
            </span>
            {r.run_attempt > 1 ? <span>attempt #{r.run_attempt}</span> : null}
          </div>
          {r.error ? <p style={{ color: 'var(--fgColor-danger)', margin: '8px 0 0' }}>{r.error}</p> : null}
        </div>
        {repo.permissions?.push ? (
          <div style={{ display: 'flex', gap: 8 }}>
            {r.status !== 'completed' ? (
              <Button size="small" leadingVisual={XIcon} onClick={() => cancel.mutate()} loading={cancel.isPending}>
                Cancel run
              </Button>
            ) : (
              <>
                {r.conclusion === 'failure' ? (
                  <Button size="small" leadingVisual={SyncIcon} onClick={() => rerun.mutate(true)} loading={rerun.isPending}>
                    Re-run failed jobs
                  </Button>
                ) : null}
                <Button size="small" leadingVisual={SyncIcon} onClick={() => rerun.mutate(false)} loading={rerun.isPending}>
                  Re-run all jobs
                </Button>
              </>
            )}
          </div>
        ) : null}
      </div>
      <div className="run-layout">
        <aside className="jobs-side">
          <div className="muted small" style={{ padding: '4px 8px' }}>Jobs</div>
          {list.map((j) => (
            <Link key={j.id} to="/$owner/$repo/actions/runs/$id" params={{ ...p, id }} search={{ job: j.id }} className={`job-link${selected?.id === j.id ? ' active' : ''}`}>
              <StatusIcon status={j.status} conclusion={j.conclusion} /> <span style={{ flex: 1 }}>{j.name}</span>
              <span className="muted small">{j.status === 'completed' || j.status === 'in_progress' ? runDuration(j) : ''}</span>
            </Link>
          ))}
          {list.length === 0 ? <p className="muted small" style={{ padding: 8 }}>No jobs.</p> : null}
        </aside>
        <div>{selected ? <JobView full={repo.full_name} job={selected} /> : null}</div>
      </div>
    </div>
  );
}

function JobView({ full, job }: { full: string; job: Job }) {
  const live = job.status !== 'completed';
  const log = useQuery(jobLogQuery(full, job.id, live));
  const groups = splitLog(log.data ?? '');
  const [open, setOpen] = useState<Record<number, boolean>>({});
  const stepsFromLog = job.steps.length ? job.steps : groups.map((g, i) => ({ name: g.name, status: 'completed' as const, conclusion: null, number: i + 1, started_at: null, completed_at: null }));
  return (
    <div className="job-view">
      <div className="job-head">
        <StatusIcon status={job.status} conclusion={job.conclusion} size={20} />
        <strong style={{ fontSize: 16 }}>{job.name}</strong>
        <span className="muted small">{statusLabel(job.status, job.conclusion)}{job.runner_name ? ` · ${job.runner_name}` : ''}{job.labels.length ? ` · ${job.labels.join(', ')}` : ''}</span>
      </div>
      {job.status === 'queued' ? (
        <div className="job-waiting muted">
          Waiting for a runner with labels <strong>{job.labels.join(', ')}</strong> to pick up this job…
        </div>
      ) : null}
      {job.status === 'waiting' ? <div className="job-waiting muted">Waiting for dependencies.</div> : null}
      <div className="steps">
        {stepsFromLog.map((st, i) => {
          const g = groups.find((x) => x.name === st.name) ?? groups[i];
          const isOpen = open[i] ?? (st.conclusion === 'failure' || (live && st.status === 'in_progress'));
          return (
            <div key={i} className="step">
              <button className="step-head" onClick={() => setOpen({ ...open, [i]: !isOpen })}>
                {isOpen ? <ChevronDownIcon /> : <ChevronRightIcon />}
                <StatusIcon status={st.status} conclusion={st.conclusion} />
                <span style={{ flex: 1 }}>{st.name}</span>
                <span className="muted small">{st.started_at ? runDuration({ started_at: st.started_at, completed_at: st.completed_at, status: st.status }) : ''}</span>
              </button>
              {isOpen && g ? (
                <pre className="step-log">
                  {g.lines.map((l, n) => (
                    <div key={n} className="log-line">
                      <span className="ln">{n + 1}</span>
                      <span className={l.startsWith('##[error]') ? 'err' : l.startsWith('##[warning]') ? 'warn' : ''}>{l.replace(/^##\[(error|warning|notice|debug|command)\]/, '')}</span>
                    </div>
                  ))}
                  {g.lines.length === 0 ? <div className="log-line muted">No output</div> : null}
                </pre>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
