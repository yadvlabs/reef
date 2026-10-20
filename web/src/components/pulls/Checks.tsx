// The Checks tab: check runs (Actions jobs) and commit statuses for the head commit.
import { RelativeTime } from '@primer/react';
import { Blankslate } from '@primer/react/experimental';
import { CheckCircleFillIcon, DotFillIcon, SkipIcon, XCircleFillIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import type { Repo } from '../../lib/api';
import { checkRunsQuery, combinedStatusQuery, type CheckRun, type CommitStatus, type Pull } from '../../lib/pulls';
import { Loading } from '../Loading';

function RunIcon({ status, conclusion }: { status: string; conclusion: string | null }) {
  if (status !== 'completed') return <DotFillIcon fill="var(--fgColor-attention)" className={status === 'in_progress' ? 'pulse-dot' : ''} />;
  if (conclusion === 'success') return <CheckCircleFillIcon fill="var(--fgColor-success)" />;
  if (conclusion === 'skipped' || conclusion === 'neutral') return <SkipIcon fill="var(--fgColor-muted)" />;
  return <XCircleFillIcon fill="var(--fgColor-danger)" />;
}

function StatusIcon({ state }: { state: CommitStatus['state'] }) {
  if (state === 'success') return <CheckCircleFillIcon fill="var(--fgColor-success)" />;
  if (state === 'pending') return <DotFillIcon fill="var(--fgColor-attention)" />;
  return <XCircleFillIcon fill="var(--fgColor-danger)" />;
}

function duration(a: string | null, b: string | null): string {
  if (!a) return '';
  const ms = (b ? Date.parse(b) : Date.now()) - Date.parse(a);
  const s = Math.max(0, Math.round(ms / 1000));
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

export function PullChecks({ repo, pr }: { repo: Repo; pr: Pull }) {
  const runs = useQuery(checkRunsQuery(repo.full_name, pr.head.sha));
  const statuses = useQuery(combinedStatusQuery(repo.full_name, pr.head.sha));
  if (runs.isPending || statuses.isPending) return <Loading />;
  const list: CheckRun[] = runs.data?.check_runs ?? [];
  const st: CommitStatus[] = statuses.data?.statuses ?? [];
  if (list.length === 0 && st.length === 0) {
    return (
      <Blankslate border>
        <Blankslate.Heading>No checks reported for this pull request</Blankslate.Heading>
        <Blankslate.Description>
          Add a workflow under <code>.github/workflows</code> to run checks on every push, or post a commit status through the API.
        </Blankslate.Description>
      </Blankslate>
    );
  }
  const groups = new Map<string, CheckRun[]>();
  for (const r of list) {
    const app = r.app?.name === 'GitHub Actions' ? 'Actions' : (r.app?.name ?? 'Checks');
    groups.set(app, [...(groups.get(app) ?? []), r]);
  }
  const failed = list.filter((r) => r.status === 'completed' && !['success', 'skipped', 'neutral'].includes(r.conclusion ?? '')).length + st.filter((s) => s.state === 'failure' || s.state === 'error').length;
  const pending = list.filter((r) => r.status !== 'completed').length + st.filter((s) => s.state === 'pending').length;
  const total = list.length + st.length;
  return (
    <div className="checks-page">
      <div className="checks-summary">
        <strong>{failed ? `${failed} failing` : pending ? `${pending} in progress` : 'All checks passed'}</strong>
        <span className="muted">
          {' '}
          · {total - failed - pending} successful, {failed} failing, {pending} pending
        </span>
      </div>
      {[...groups.entries()].map(([app, rs]) => (
        <div key={app} className="checks-group">
          <div className="checks-group-head">{app}</div>
          {rs.map((r) => (
            <a key={r.id} href={r.html_url} className="check-row">
              <RunIcon status={r.status} conclusion={r.conclusion} />
              <span className="check-name">{r.name}</span>
              <span className="muted small">
                {r.status !== 'completed' ? (r.status === 'in_progress' ? 'In progress' : 'Queued') : `${r.conclusion === 'success' ? 'Successful' : r.conclusion === 'skipped' ? 'Skipped' : 'Failed'} in ${duration(r.started_at, r.completed_at)}`}
                {r.completed_at ? (
                  <>
                    {' '}
                    · <RelativeTime datetime={r.completed_at} />
                  </>
                ) : null}
              </span>
              <span className="details">Details</span>
            </a>
          ))}
        </div>
      ))}
      {st.length ? (
        <div className="checks-group">
          <div className="checks-group-head">Commit statuses</div>
          {st.map((s) => (
            <a key={s.context} href={s.target_url ?? '#'} className="check-row">
              <StatusIcon state={s.state} />
              <span className="check-name">{s.context}</span>
              <span className="muted small">
                {s.description ?? s.state} · <RelativeTime datetime={s.created_at} />
              </span>
              {s.target_url ? <span className="details">Details</span> : null}
            </a>
          ))}
        </div>
      ) : null}
    </div>
  );
}
