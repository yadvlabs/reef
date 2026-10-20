// The contribution calendar and activity timeline on a user profile, like GitHub's.
import { ActionList } from '@primer/react';
import { GitCommitIcon, GitPullRequestIcon, IssueOpenedIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { useState } from 'react';
import { contributionsQuery, fmtInt, type ContributionDay } from '../lib/insights';
import { Loading } from './Loading';

const DAY_LABELS = ['', 'Mon', '', 'Wed', '', 'Fri', ''];
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

function level(count: number, max: number): number {
  if (count <= 0) return 0;
  if (max <= 4) return Math.min(4, count);
  const q = count / max;
  return q > 0.75 ? 4 : q > 0.5 ? 3 : q > 0.25 ? 2 : 1;
}

function iso(d: Date): string {
  return d.toISOString().slice(0, 10);
}

/** Calendar columns: weeks starting on Sunday, padded so the first week is complete. */
function weeksOf(days: ContributionDay[]): (ContributionDay | null)[][] {
  if (days.length === 0) return [];
  const firstDow = new Date(days[0].date + 'T00:00:00Z').getUTCDay();
  const cells: (ContributionDay | null)[] = [...Array<null>(firstDow).fill(null), ...days];
  const weeks: (ContributionDay | null)[][] = [];
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7));
  return weeks;
}

export function ContributionGraph({ login, since }: { login: string; since?: string }) {
  const firstYear = since ? new Date(since).getUTCFullYear() : new Date().getUTCFullYear();
  const thisYear = new Date().getUTCFullYear();
  const [year, setYear] = useState<number | null>(null);
  const range = year === null ? {} : { from: `${year}-01-01`, to: year === thisYear ? iso(new Date()) : `${year}-12-31` };
  const q = useQuery(contributionsQuery(login, range.from, range.to));
  const years: number[] = [];
  for (let y = thisYear; y >= Math.min(firstYear, thisYear); y--) years.push(y);
  if (q.isPending) return <Loading />;
  if (q.isError) return null;
  const d = q.data;
  const max = Math.max(0, ...d.days.map((x) => x.count));
  const weeks = weeksOf(d.days);
  const monthLabels: { col: number; label: string }[] = [];
  let lastMonth = -1;
  weeks.forEach((w, i) => {
    const first = w.find((c) => c);
    if (!first) return;
    const m = new Date(first.date + 'T00:00:00Z').getUTCMonth();
    if (m !== lastMonth) {
      if (monthLabels.length === 0 || i - monthLabels[monthLabels.length - 1].col > 2) monthLabels.push({ col: i, label: MONTHS[m] });
      lastMonth = m;
    }
  });
  const cell = 11;
  const gap = 3;
  const left = 30;
  const top = 16;
  const width = left + weeks.length * (cell + gap);
  const height = top + 7 * (cell + gap);
  return (
    <div className="contrib-wrap">
      <div className="contrib-graph">
        <h2 className="overview-title">
          {fmtInt(d.total)} {d.total === 1 ? 'contribution' : 'contributions'} in {year === null ? 'the last year' : year}
        </h2>
        <div className="contrib-calendar">
          <svg width={width} height={height} className="contrib-svg" role="img" aria-label="Contribution calendar">
            {monthLabels.map((m) => (
              <text key={m.col} x={left + m.col * (cell + gap)} y={10} className="contrib-label">
                {m.label}
              </text>
            ))}
            {DAY_LABELS.map((l, i) => (l ? <text key={i} x={0} y={top + i * (cell + gap) + cell - 2} className="contrib-label">{l}</text> : null))}
            {weeks.map((w, x) =>
              w.map((c, y) =>
                c ? (
                  <rect key={c.date} x={left + x * (cell + gap)} y={top + y * (cell + gap)} width={cell} height={cell} rx={2} data-level={level(c.count, max)} className="contrib-day">
                    <title>{`${c.count === 0 ? 'No' : fmtInt(c.count)} ${c.count === 1 ? 'contribution' : 'contributions'} on ${new Date(c.date + 'T00:00:00Z').toLocaleDateString(undefined, { month: 'long', day: 'numeric', year: 'numeric', timeZone: 'UTC' })}`}</title>
                  </rect>
                ) : null,
              ),
            )}
          </svg>
          <div className="contrib-legend muted small">
            <span>Learn how we count contributions</span>
            <span style={{ flex: 1 }} />
            <span>Less</span>
            {[0, 1, 2, 3, 4].map((l) => (
              <span key={l} className="contrib-day legend" data-level={l} />
            ))}
            <span>More</span>
          </div>
        </div>
      </div>
      {years.length > 1 ? (
        <div className="contrib-years">
          <ActionList selectionVariant="single">
            {years.map((y) => (
              <ActionList.Item key={y} selected={(year ?? thisYear) === y} onSelect={() => setYear(y === thisYear ? null : y)}>
                {y}
              </ActionList.Item>
            ))}
          </ActionList>
        </div>
      ) : null}
      <ContributionActivity login={login} from={range.from} to={range.to} />
    </div>
  );
}

function ContributionActivity({ login, from, to }: { login: string; from?: string; to?: string }) {
  const q = useQuery(contributionsQuery(login, from, to));
  const months = q.data?.months ?? [];
  if (!q.data) return null;
  return (
    <div className="contrib-activity">
      <h2 className="overview-title">Contribution activity</h2>
      {months.length === 0 ? (
        <p className="muted">{login} has no activity yet for this period.</p>
      ) : (
        months.map((m) => {
          const date = new Date(m.month + '-01T00:00:00Z');
          const label = date.toLocaleDateString(undefined, { month: 'long', year: 'numeric', timeZone: 'UTC' });
          const commits = m.commits.reduce((n, r) => n + r.count, 0);
          const issues = m.issues.reduce((n, r) => n + r.count, 0);
          const pulls = m.pulls.reduce((n, r) => n + r.count, 0);
          return (
            <section key={m.month} className="activity-month">
              <h3>{label}</h3>
              {commits ? (
                <ActivityGroup icon={<GitCommitIcon />} title={`Created ${fmtInt(commits)} ${commits === 1 ? 'commit' : 'commits'} in ${m.commits.length} ${m.commits.length === 1 ? 'repository' : 'repositories'}`}>
                  {m.commits.map((r) => (
                    <li key={r.repo}>
                      <RepoLink full={r.repo} />
                      <span className="muted small">{fmtInt(r.count)} {r.count === 1 ? 'commit' : 'commits'}</span>
                    </li>
                  ))}
                </ActivityGroup>
              ) : null}
              {pulls ? (
                <ActivityGroup icon={<GitPullRequestIcon />} title={`Opened ${fmtInt(pulls)} ${pulls === 1 ? 'pull request' : 'pull requests'} in ${m.pulls.length} ${m.pulls.length === 1 ? 'repository' : 'repositories'}`}>
                  {m.pulls.map((r) => (
                    <li key={r.repo}>
                      <RepoLink full={r.repo} />
                      <span className="muted small">{fmtInt(r.count)} {r.count === 1 ? 'pull request' : 'pull requests'}</span>
                    </li>
                  ))}
                </ActivityGroup>
              ) : null}
              {issues ? (
                <ActivityGroup icon={<IssueOpenedIcon />} title={`Opened ${fmtInt(issues)} ${issues === 1 ? 'issue' : 'issues'} in ${m.issues.length} ${m.issues.length === 1 ? 'repository' : 'repositories'}`}>
                  {m.issues.map((r) => (
                    <li key={r.repo}>
                      <RepoLink full={r.repo} />
                      <span className="muted small">{fmtInt(r.count)} {r.count === 1 ? 'issue' : 'issues'}</span>
                    </li>
                  ))}
                </ActivityGroup>
              ) : null}
            </section>
          );
        })
      )}
    </div>
  );
}

function RepoLink({ full }: { full: string }) {
  const [owner, repo] = full.split('/');
  return (
    <Link to="/$owner/$repo" params={{ owner, repo }}>
      {full}
    </Link>
  );
}

function ActivityGroup({ icon, title, children }: { icon: React.ReactNode; title: string; children: React.ReactNode }) {
  return (
    <div className="activity-group">
      <div className="activity-title">
        <span className="activity-icon">{icon}</span>
        <strong>{title}</strong>
      </div>
      <ul>{children}</ul>
    </div>
  );
}
