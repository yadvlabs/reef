import { Avatar, RelativeTime } from '@primer/react';
import { PlayIcon, GitBranchIcon, StopwatchIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { StatusIcon } from '../components/ActionsBits';
import { ButtonLink } from '../components/links';
import { Loading } from '../components/Loading';
import { runDuration, runsQuery, workflowsQuery } from '../lib/actions';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/actions/')({
  validateSearch: (s: Record<string, unknown>): { workflow?: number; branch?: string } => ({ ...(typeof s.workflow === 'number' ? { workflow: s.workflow } : {}), ...(typeof s.branch === 'string' ? { branch: s.branch } : {}) }),
  component: ActionsHome,
});

function ActionsHome() {
  const repo = useRepo();
  const { workflow, branch } = Route.useSearch();
  const workflows = useQuery(workflowsQuery(repo.full_name));
  const runs = useQuery(runsQuery(repo.full_name, workflow, branch));
  const p = { owner: repo.owner.login, repo: repo.name };
  const current = workflows.data?.workflows.find((w) => w.id === workflow);
  return (
    <div className="actions-layout">
      <aside className="actions-side">
        <Link to="/$owner/$repo/actions" params={p} search={{}} className={`wf-link${!workflow ? ' active' : ''}`}>
          All workflows
        </Link>
        <div className="muted small" style={{ padding: '8px 8px 4px' }}>Workflows</div>
        {workflows.data?.workflows.map((w) => (
          <Link key={w.id} to="/$owner/$repo/actions" params={p} search={{ workflow: w.id }} className={`wf-link${workflow === w.id ? ' active' : ''}${w.state !== 'active' ? ' muted' : ''}`}>
            {w.name}
          </Link>
        ))}
        {workflows.data?.workflows.length === 0 ? <p className="muted small" style={{ padding: 8 }}>No workflows in <code>.github/workflows</code> on {repo.default_branch}.</p> : null}
        {repo.permissions?.admin ? (
          <div style={{ padding: 8, marginTop: 8 }}>
            <ButtonLink to="/$owner/$repo/settings/actions" params={p} size="small" block>
              Runners and secrets
            </ButtonLink>
          </div>
        ) : null}
      </aside>
      <div>
        <div className="actions-head">
          <h2>{current ? current.name : 'All workflows'}</h2>
          {current ? <span className="muted small">{current.path}</span> : null}
          <span style={{ flex: 1 }} />
          <span className="muted small">{runs.data?.total_count ?? 0} workflow runs</span>
        </div>
        {runs.isPending ? <Loading /> : null}
        {runs.data?.workflow_runs.length === 0 ? (
          <Blankslate spacious border>
            <Blankslate.Visual>
              <PlayIcon size="medium" />
            </Blankslate.Visual>
            <Blankslate.Heading>{workflows.data?.workflows.length ? 'No runs yet' : 'Get started with Actions'}</Blankslate.Heading>
            <Blankslate.Description>
              {workflows.data?.workflows.length ? 'Runs appear here when a workflow is triggered by a push, a pull request, or manually.' : 'Add a workflow file under .github/workflows and register a runner in the repository settings. Stock actions/runner binaries work unchanged.'}
            </Blankslate.Description>
          </Blankslate>
        ) : null}
        <div className="run-list">
          {runs.data?.workflow_runs.map((r) => (
            <div key={r.id} className="run-row">
              <StatusIcon status={r.status} conclusion={r.conclusion} />
              <div className="run-main">
                <Link to="/$owner/$repo/actions/runs/$id" params={{ ...p, id: String(r.id) }} search={{}} className="run-title">
                  {r.display_title}
                </Link>
                <div className="muted small">
                  {r.name} #{r.run_number}: {r.event === 'pull_request' ? 'Pull request' : r.event === 'workflow_dispatch' ? 'Manually run' : 'Commit'} <code>{r.head_sha.slice(0, 7)}</code> {r.event === 'push' ? 'pushed' : r.event === 'pull_request' ? 'synchronized' : 'triggered'} by {r.triggering_actor?.login ?? r.actor?.login ?? 'unknown'}
                  {r.error ? <span style={{ color: 'var(--fgColor-danger)' }}> · {r.error}</span> : null}
                </div>
              </div>
              <div className="run-meta">
                {r.head_branch ? (
                  <Link to="/$owner/$repo/actions" params={p} search={{ workflow, branch: r.head_branch }} className="branch-pill">
                    <GitBranchIcon /> {r.head_branch}
                  </Link>
                ) : null}
                {r.actor ? <Avatar src={r.actor.avatar_url} size={20} alt="" /> : null}
              </div>
              <div className="run-time muted small">
                <span>
                  <RelativeTime datetime={r.created_at} />
                </span>
                <span>
                  <StopwatchIcon /> {runDuration(r)}
                </span>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
