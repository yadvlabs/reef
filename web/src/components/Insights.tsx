// Repository insights: pulse, contributors, commits, code frequency and forks, laid out
// like GitHub's Insights tab (left navigation, content on the right).
import { ActionList, ActionMenu, Avatar, NavList, RelativeTime } from '@primer/react';
import { Blankslate } from '@primer/react/experimental';
import { Crusty } from './Crusty';
import { GitPullRequestIcon, IssueOpenedIcon, RepoForkedIcon, StarIcon, TriangleDownIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { Link, useRouterState } from '@tanstack/react-router';
import { useMemo, useState } from 'react';
import { api, type Repo } from '../lib/api';
import { codeFrequencyQuery, commitActivityQuery, contributorStatsQuery, fmtInt, pulseQuery, punchCardQuery, sinceWeeks, type CodeFrequency, type ContributorStats, type PulseItem } from '../lib/insights';
import { ErrorFlash, Loading } from './Loading';
import { NavListLink } from './links';

export function InsightsLayout({ repo, children }: { repo: Repo; children: React.ReactNode }) {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const base = `/${repo.owner.login}/${repo.name}`;
  const p = { owner: repo.owner.login, repo: repo.name };
  const is = (suffix: string) => (path === base + suffix ? 'page' : undefined);
  return (
    <div className="insights-layout">
      <NavList aria-label="Insights">
        <NavListLink to="/$owner/$repo/pulse" params={p} aria-current={is('/pulse')}>
          Pulse
        </NavListLink>
        <NavListLink to="/$owner/$repo/graphs/contributors" params={p} aria-current={is('/graphs/contributors')}>
          Contributors
        </NavListLink>
        <NavListLink to="/$owner/$repo/graphs/commit-activity" params={p} aria-current={is('/graphs/commit-activity')}>
          Commits
        </NavListLink>
        <NavListLink to="/$owner/$repo/graphs/code-frequency" params={p} aria-current={is('/graphs/code-frequency')}>
          Code frequency
        </NavListLink>
        <NavListLink to="/$owner/$repo/forks" params={p} aria-current={is('/forks')}>
          Forks
        </NavListLink>
      </NavList>
      <div className="insights-main">{children}</div>
    </div>
  );
}

// ----- shared bits -----

/** A plain bar chart in SVG. Values may be negative (drawn below the baseline). */
function Bars({ series, height = 120, color = 'var(--fgColor-success)', negColor = 'var(--fgColor-danger)', labels }: { series: number[]; height?: number; color?: string; negColor?: string; labels?: string[] }) {
  const w = 720;
  const n = Math.max(series.length, 1);
  const bw = Math.max(1, w / n - 1);
  const maxPos = Math.max(0, ...series);
  const maxNeg = Math.max(0, ...series.map((v) => -v));
  const total = maxPos + maxNeg || 1;
  const zero = (maxPos / total) * height;
  return (
    <svg viewBox={`0 0 ${w} ${height}`} className="chart" preserveAspectRatio="none" role="img" aria-label="bar chart">
      <line x1={0} x2={w} y1={zero} y2={zero} stroke="var(--borderColor-default)" />
      {series.map((v, i) => {
        const h = (Math.abs(v) / total) * height;
        const x = (i * w) / n;
        return <rect key={i} x={x} y={v >= 0 ? zero - h : zero} width={bw} height={h} fill={v >= 0 ? color : negColor}>{labels ? <title>{`${labels[i]}: ${fmtInt(v)}`}</title> : null}</rect>;
      })}
    </svg>
  );
}

const weekLabel = (w: number) => new Date(w * 1000).toISOString().slice(0, 10);

/** Prepend empty weeks so short histories still get a readable time axis. */
function padWeeks(keys: number[], min: number): number[] {
  if (keys.length === 0 || keys.length >= min) return keys;
  const out = [...keys];
  while (out.length < min) out.unshift(out[0] - 7 * 86400);
  return out;
}

function Card({ title, children, right }: { title?: React.ReactNode; children: React.ReactNode; right?: React.ReactNode }) {
  return (
    <section className="insight-card">
      {title ? (
        <header>
          <h3>{title}</h3>
          <span style={{ flex: 1 }} />
          {right}
        </header>
      ) : null}
      <div className="body">{children}</div>
    </section>
  );
}

// ----- pulse -----

const PERIODS = [
  { days: 1, label: '24 hours' },
  { days: 3, label: '3 days' },
  { days: 7, label: '1 week' },
  { days: 30, label: '1 month' },
];

function ItemList({ items, kind, repo }: { items: PulseItem[]; kind: 'pull' | 'issue'; repo: Repo }) {
  if (items.length === 0) return null;
  return (
    <ul className="pulse-items">
      {items.map((i) => (
        <li key={i.number}>
          {kind === 'pull' ? <GitPullRequestIcon fill={i.merged_at ? 'var(--fgColor-done)' : 'var(--fgColor-success)'} /> : <IssueOpenedIcon fill={i.closed_at ? 'var(--fgColor-done)' : 'var(--fgColor-success)'} />}
          <Link to={kind === 'pull' ? '/$owner/$repo/pull/$number' : '/$owner/$repo/issues/$number'} params={{ owner: repo.owner.login, repo: repo.name, number: String(i.number) }}>
            {i.title}
          </Link>
          <span className="muted small">
            #{i.number} · {i.user.login} · <RelativeTime datetime={i.merged_at ?? i.closed_at ?? i.created_at} />
          </span>
        </li>
      ))}
    </ul>
  );
}

export function PulsePage({ repo }: { repo: Repo }) {
  const [days, setDays] = useState(7);
  const q = useQuery(pulseQuery(repo.owner.login, repo.name, days));
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const d = q.data;
  const period = PERIODS.find((p) => p.days === days)?.label ?? `${days} days`;
  const heading = { 1: 'Last 24 hours', 3: 'Last 3 days', 7: 'Last week', 30: 'Last month' }[days] ?? `Last ${days} days`;
  const prTotal = d.counts.merged_pulls + d.counts.open_pulls;
  const issueTotal = d.counts.closed_issues + d.counts.new_issues;
  return (
    <>
      <div className="insights-head">
        <h2>{heading}</h2>
        <span className="muted small">
          {new Date(d.since).toLocaleDateString()} – {new Date(d.until).toLocaleDateString()}
        </span>
        <span style={{ flex: 1 }} />
        <ActionMenu>
          <ActionMenu.Button size="small" trailingAction={TriangleDownIcon}>
            Period: {period}
          </ActionMenu.Button>
          <ActionMenu.Overlay>
            <ActionList selectionVariant="single">
              {PERIODS.map((p) => (
                <ActionList.Item key={p.days} selected={p.days === days} onSelect={() => setDays(p.days)}>
                  {p.label}
                </ActionList.Item>
              ))}
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
      </div>
      <Card title="Overview">
        <div className="pulse-overview">
          <div>
            <div className="pulse-bar">
              <span className="done" style={{ width: prTotal ? `${(d.counts.merged_pulls / prTotal) * 100}%` : 0 }} />
              <span className="open" style={{ width: prTotal ? `${(d.counts.open_pulls / prTotal) * 100}%` : 0 }} />
            </div>
            <div className="pulse-counts">
              <span>
                <strong>{d.counts.active_pulls}</strong> Active pull requests
              </span>
            </div>
            <div className="pulse-counts small">
              <span>
                <GitPullRequestIcon fill="var(--fgColor-done)" /> <strong>{d.counts.merged_pulls}</strong> Merged pull requests
              </span>
              <span>
                <GitPullRequestIcon fill="var(--fgColor-success)" /> <strong>{d.counts.open_pulls}</strong> Open pull requests
              </span>
            </div>
          </div>
          <div>
            <div className="pulse-bar">
              <span className="done" style={{ width: issueTotal ? `${(d.counts.closed_issues / issueTotal) * 100}%` : 0 }} />
              <span className="open" style={{ width: issueTotal ? `${(d.counts.new_issues / issueTotal) * 100}%` : 0 }} />
            </div>
            <div className="pulse-counts">
              <span>
                <strong>{d.counts.active_issues}</strong> Active issues
              </span>
            </div>
            <div className="pulse-counts small">
              <span>
                <IssueOpenedIcon fill="var(--fgColor-done)" /> <strong>{d.counts.closed_issues}</strong> Closed issues
              </span>
              <span>
                <IssueOpenedIcon fill="var(--fgColor-success)" /> <strong>{d.counts.new_issues}</strong> New issues
              </span>
            </div>
          </div>
        </div>
        <p style={{ marginTop: 16 }}>
          Excluding merges, <strong>{d.commits.authors}</strong> {d.commits.authors === 1 ? 'author has' : 'authors have'} pushed <strong>{d.commits.count}</strong> {d.commits.count === 1 ? 'commit' : 'commits'} to{' '}
          <code>{repo.default_branch}</code>. On <code>{repo.default_branch}</code>, <strong>{fmtInt(d.commits.files_changed)}</strong> files have changed and there have been{' '}
          <span style={{ color: 'var(--fgColor-success)' }}>{fmtInt(d.commits.additions)} additions</span> and <span style={{ color: 'var(--fgColor-danger)' }}>{fmtInt(d.commits.deletions)} deletions</span>.
        </p>
        {d.authors.length ? (
          <div className="pulse-authors">
            {d.authors.slice(0, 20).map((a, i) => (
              <span key={i} title={`${a.author.login ?? a.author.name} · ${a.commits} commits`}>
                <Avatar src={a.author.avatar_url ?? ''} size={24} alt="" />
              </span>
            ))}
          </div>
        ) : null}
      </Card>
      {d.merged_pulls.length ? (
        <Card title={`${d.counts.merged_pulls} Pull ${d.counts.merged_pulls === 1 ? 'request' : 'requests'} merged by ${new Set(d.merged_pulls.map((p) => p.user.login)).size} ${new Set(d.merged_pulls.map((p) => p.user.login)).size === 1 ? 'person' : 'people'}`}>
          <ItemList items={d.merged_pulls} kind="pull" repo={repo} />
        </Card>
      ) : null}
      {d.open_pulls.length ? (
        <Card title={`${d.counts.open_pulls} Pull ${d.counts.open_pulls === 1 ? 'request' : 'requests'} proposed by ${new Set(d.open_pulls.map((p) => p.user.login)).size} ${new Set(d.open_pulls.map((p) => p.user.login)).size === 1 ? 'person' : 'people'}`}>
          <ItemList items={d.open_pulls} kind="pull" repo={repo} />
        </Card>
      ) : null}
      {d.closed_issues.length ? (
        <Card title={`${d.counts.closed_issues} ${d.counts.closed_issues === 1 ? 'Issue' : 'Issues'} closed by ${new Set(d.closed_issues.map((p) => p.user.login)).size} ${new Set(d.closed_issues.map((p) => p.user.login)).size === 1 ? 'person' : 'people'}`}>
          <ItemList items={d.closed_issues} kind="issue" repo={repo} />
        </Card>
      ) : null}
      {d.new_issues.length ? (
        <Card title={`${d.counts.new_issues} ${d.counts.new_issues === 1 ? 'Issue' : 'Issues'} created by ${new Set(d.new_issues.map((p) => p.user.login)).size} ${new Set(d.new_issues.map((p) => p.user.login)).size === 1 ? 'person' : 'people'}`}>
          <ItemList items={d.new_issues} kind="issue" repo={repo} />
        </Card>
      ) : null}
      {!d.merged_pulls.length && !d.open_pulls.length && !d.closed_issues.length && !d.new_issues.length && !d.commits.count ? (
        <Blankslate>
          <Blankslate.Visual>
            <Crusty mood="resting" size={120} />
          </Blankslate.Visual>
          <Blankslate.Heading>No activity in this period</Blankslate.Heading>
          <Blankslate.Description>Try a longer period from the menu above.</Blankslate.Description>
        </Blankslate>
      ) : null}
    </>
  );
}

// ----- contributors -----

const CONTRIB_PERIODS: { months: number | null; label: string }[] = [
  { months: null, label: 'All time' },
  { months: 3, label: 'Last 3 months' },
  { months: 6, label: 'Last 6 months' },
  { months: 12, label: 'Last 12 months' },
  { months: 24, label: 'Last 24 months' },
];

type Metric = 'c' | 'a' | 'd';

export function ContributorsPage({ repo }: { repo: Repo }) {
  const q = useQuery(contributorStatsQuery(repo.owner.login, repo.name));
  const [months, setMonths] = useState<number | null>(null);
  const [metric, setMetric] = useState<Metric>('c');
  const since = sinceWeeks(months);
  const rows = useMemo(() => {
    const list = (q.data ?? []).map((c: ContributorStats) => {
      const weeks = c.weeks.filter((w) => w.w >= since);
      const sum = (k: Metric) => weeks.reduce((n, w) => n + w[k], 0);
      return { author: c.author, weeks, c: sum('c'), a: sum('a'), d: sum('d') };
    });
    return list.filter((r) => r.c > 0).sort((x, y) => y[metric] - x[metric]);
  }, [q.data, since, metric]);
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const weekKeys = padWeeks(rows[0]?.weeks.map((w) => w.w) ?? [], 12);
  const totals = weekKeys.map((w) => rows.reduce((n, r) => n + (r.weeks.find((x) => x.w === w)?.[metric] ?? 0), 0));
  const total = rows.reduce((n, r) => n + r[metric], 0);
  const first = weekKeys[0];
  const last = weekKeys[weekKeys.length - 1];
  const label = { c: 'Commits', a: 'Additions', d: 'Deletions' }[metric];
  return (
    <>
      <div className="insights-head">
        <h2>Contributors</h2>
        <span style={{ flex: 1 }} />
        <ActionMenu>
          <ActionMenu.Button size="small" trailingAction={TriangleDownIcon}>
            Contributions: {label}
          </ActionMenu.Button>
          <ActionMenu.Overlay>
            <ActionList selectionVariant="single">
              {(['c', 'a', 'd'] as Metric[]).map((m) => (
                <ActionList.Item key={m} selected={m === metric} onSelect={() => setMetric(m)}>
                  {{ c: 'Commits', a: 'Additions', d: 'Deletions' }[m]}
                </ActionList.Item>
              ))}
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
        <ActionMenu>
          <ActionMenu.Button size="small" trailingAction={TriangleDownIcon}>
            Period: {CONTRIB_PERIODS.find((p) => p.months === months)?.label}
          </ActionMenu.Button>
          <ActionMenu.Overlay>
            <ActionList selectionVariant="single">
              {CONTRIB_PERIODS.map((p) => (
                <ActionList.Item key={String(p.months)} selected={p.months === months} onSelect={() => setMonths(p.months)}>
                  {p.label}
                </ActionList.Item>
              ))}
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
      </div>
      {rows.length === 0 ? (
        <Blankslate border>
          <Blankslate.Heading>No contributions in this period</Blankslate.Heading>
          <Blankslate.Description>Nothing was committed to {repo.default_branch} in the selected period.</Blankslate.Description>
        </Blankslate>
      ) : (
        <>
          <Card title={first ? `${weekLabel(first)} – ${weekLabel(last)}` : undefined} right={<span className="muted small">{label.toLowerCase()} to {repo.default_branch}, excluding merge commits</span>}>
            <div className="chart-total">{fmtInt(total)} {label.toLowerCase()}</div>
            <Bars series={totals} height={140} color={metric === 'd' ? 'var(--fgColor-danger)' : 'var(--fgColor-success)'} labels={weekKeys.map(weekLabel)} />
          </Card>
          <div className="contrib-grid">
            {rows.map((r, i) => (
              <section key={i} className="insight-card contrib-card">
                <header>
                  <Avatar src={r.author.avatar_url ?? ''} size={40} alt="" />
                  <div>
                    <div>
                      {r.author.login ? (
                        <Link to="/$owner" params={{ owner: r.author.login }}>
                          <strong>{r.author.login}</strong>
                        </Link>
                      ) : (
                        <strong>{r.author.name ?? r.author.email}</strong>
                      )}
                    </div>
                    <div className="muted small">
                      {fmtInt(r.c)} {r.c === 1 ? 'commit' : 'commits'} <span style={{ color: 'var(--fgColor-success)' }}>{fmtInt(r.a)} ++</span> <span style={{ color: 'var(--fgColor-danger)' }}>{fmtInt(r.d)} --</span>
                    </div>
                  </div>
                  <span className="rank">#{i + 1}</span>
                </header>
                <div className="body">
                  <Bars series={weekKeys.map((w) => r.weeks.find((x) => x.w === w)?.[metric] ?? 0)} height={80} color={metric === 'd' ? 'var(--fgColor-danger)' : 'var(--fgColor-accent)'} labels={weekKeys.map(weekLabel)} />
                </div>
              </section>
            ))}
          </div>
        </>
      )}
    </>
  );
}

// ----- commits -----

const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

export function CommitsPage({ repo }: { repo: Repo }) {
  const q = useQuery(commitActivityQuery(repo.owner.login, repo.name));
  const punch = useQuery(punchCardQuery(repo.owner.login, repo.name));
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const weeks = q.data;
  const total = weeks.reduce((n, w) => n + w.total, 0);
  const byDay = DAYS.map((_, d) => weeks.reduce((n, w) => n + (w.days[d] ?? 0), 0));
  const maxDay = Math.max(1, ...byDay);
  const byHour = Array.from({ length: 24 }, (_, h) => (punch.data ?? []).filter((p) => p[1] === h).reduce((n, p) => n + p[2], 0));
  return (
    <>
      <div className="insights-head">
        <h2>Commits</h2>
      </div>
      <Card title="Commits over the last year" right={<span className="muted small">{fmtInt(total)} commits to {repo.default_branch}</span>}>
        <Bars series={weeks.map((w) => w.total)} height={140} color="var(--fgColor-success)" labels={weeks.map((w) => weekLabel(w.week))} />
        <div className="chart-axis muted small">
          <span>{weeks.length ? weekLabel(weeks[0].week) : ''}</span>
          <span>{weeks.length ? weekLabel(weeks[weeks.length - 1].week) : ''}</span>
        </div>
      </Card>
      <Card title="Average commits per day of week">
        <div className="dow">
          {DAYS.map((d, i) => (
            <div key={d} className="dow-row">
              <span className="muted small">{d.slice(0, 3)}</span>
              <span className="dow-bar">
                <span style={{ width: `${(byDay[i] / maxDay) * 100}%` }} />
              </span>
              <span className="small">{fmtInt(byDay[i])}</span>
            </div>
          ))}
        </div>
      </Card>
      {punch.data ? (
        <Card title="Commits by hour of day (UTC)">
          <Bars series={byHour} height={100} color="var(--fgColor-accent)" labels={byHour.map((_, h) => `${h}:00`)} />
          <div className="chart-axis muted small">
            <span>00:00</span>
            <span>12:00</span>
            <span>23:00</span>
          </div>
        </Card>
      ) : null}
    </>
  );
}

// ----- code frequency -----

export function CodeFrequencyPage({ repo }: { repo: Repo }) {
  const q = useQuery(codeFrequencyQuery(repo.owner.login, repo.name));
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const rows = padRows(q.data, 12);
  const adds = rows.reduce((n, r) => n + r[1], 0);
  const dels = rows.reduce((n, r) => n + -r[2], 0);
  const series = rows.flatMap((r) => [r[1], r[2]]);
  const labels = rows.flatMap((r) => [weekLabel(r[0]), weekLabel(r[0])]);
  return (
    <>
      <div className="insights-head">
        <h2>Code frequency</h2>
      </div>
      <Card title={`Additions and deletions per week on ${repo.default_branch}`} right={<span className="small"><span style={{ color: 'var(--fgColor-success)' }}>{fmtInt(adds)} additions</span> · <span style={{ color: 'var(--fgColor-danger)' }}>{fmtInt(dels)} deletions</span></span>}>
        <Bars series={series} height={220} labels={labels} />
        <div className="chart-axis muted small">
          <span>{rows.length ? weekLabel(rows[0][0]) : ''}</span>
          <span>{rows.length ? weekLabel(rows[rows.length - 1][0]) : ''}</span>
        </div>
      </Card>
    </>
  );
}

function padRows(rows: CodeFrequency, min: number): CodeFrequency {
  if (rows.length === 0 || rows.length >= min) return rows;
  const out: CodeFrequency = [...rows];
  while (out.length < min) out.unshift([out[0][0] - 7 * 86400, 0, 0]);
  return out;
}

// ----- forks -----

export function ForksPage({ repo }: { repo: Repo }) {
  const q = useQuery({ queryKey: ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase(), 'forks'], queryFn: () => api<Repo[]>(`/repos/${repo.full_name}/forks?per_page=100`) });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  return (
    <>
      <div className="insights-head">
        <h2>Forks</h2>
        <span className="muted small">{q.data.length} {q.data.length === 1 ? 'fork' : 'forks'}</span>
      </div>
      {q.data.length === 0 ? (
        <Blankslate border>
          <Blankslate.Visual>
            <RepoForkedIcon size="medium" />
          </Blankslate.Visual>
          <Blankslate.Heading>No one has forked this repository yet</Blankslate.Heading>
          <Blankslate.Description>Forks are copies of a repository. Forking a repository allows you to freely experiment with changes without affecting the original project.</Blankslate.Description>
        </Blankslate>
      ) : (
        <div className="key-list">
          {q.data.map((f) => (
            <div key={f.id} className="item" style={{ alignItems: 'center' }}>
              <Avatar src={f.owner.avatar_url} size={24} alt="" square={f.owner.type === 'Organization'} />
              <div className="body">
                <Link to="/$owner/$repo" params={{ owner: f.owner.login, repo: f.name }}>
                  {f.owner.login} / <strong>{f.name}</strong>
                </Link>
              </div>
              <span className="muted small" style={{ display: 'flex', gap: 12 }}>
                <span>
                  <StarIcon /> {f.stargazers_count}
                </span>
                <span>
                  <RepoForkedIcon /> {f.forks_count}
                </span>
                <span>
                  Updated <RelativeTime datetime={f.updated_at} />
                </span>
              </span>
            </div>
          ))}
        </div>
      )}
    </>
  );
}
