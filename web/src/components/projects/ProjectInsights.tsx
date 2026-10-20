// Project insights: items over time by status (burn up), items per status, and any field.
import { ActionList, ActionMenu } from '@primer/react';
import { TriangleDownIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { insightsQuery, OPTION_DOTS, type ProjectFull } from '../../lib/projects';
import { ErrorFlash, Loading } from '../Loading';

const RANGES = [
  { key: '2w', label: 'Last 2 weeks' },
  { key: '1m', label: 'Last month' },
  { key: '3m', label: 'Last 3 months' },
  { key: '6m', label: 'Last 6 months' },
  { key: '1y', label: 'Last year' },
];

const colorOf = (c: string) => OPTION_DOTS[c] ?? OPTION_DOTS.GRAY;

export function ProjectInsights({ project }: { project: ProjectFull }) {
  const [range, setRange] = useState('3m');
  const [field, setField] = useState('state');
  const q = useQuery(insightsQuery(project.owner.login, project.number, range, field));
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const d = q.data;
  return (
    <div className="page insights-page">
      <div className="insights-head">
        <h2>Insights</h2>
        <span style={{ flex: 1 }} />
        <ActionMenu>
          <ActionMenu.Button size="small" trailingAction={TriangleDownIcon}>
            {RANGES.find((r) => r.key === range)?.label}
          </ActionMenu.Button>
          <ActionMenu.Overlay>
            <ActionList selectionVariant="single">
              {RANGES.map((r) => (
                <ActionList.Item key={r.key} selected={r.key === range} onSelect={() => setRange(r.key)}>
                  {r.label}
                </ActionList.Item>
              ))}
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
      </div>
      <section className="insight-card">
        <header>
          <h3>Burn up</h3>
          <span style={{ flex: 1 }} />
          <span className="muted small">
            {d.since} – {d.until} · items by status
          </span>
        </header>
        <div className="body">
          <BurnUp data={d} />
        </div>
      </section>
      <div className="insights-grid">
        <section className="insight-card">
          <header>
            <h3>Status</h3>
            <span style={{ flex: 1 }} />
            <span className="muted small">{d.status.total} items</span>
          </header>
          <div className="body">
            <Columns buckets={d.status.buckets} />
          </div>
        </section>
        <section className="insight-card">
          <header>
            <h3>{d.field?.name ?? 'Field'}</h3>
            <span style={{ flex: 1 }} />
            <ActionMenu>
              <ActionMenu.Button size="small" variant="invisible" trailingAction={TriangleDownIcon}>
                X-axis: {d.fields.find((f) => f.key === field)?.name ?? field}
              </ActionMenu.Button>
              <ActionMenu.Overlay>
                <ActionList selectionVariant="single">
                  {d.fields.map((f) => (
                    <ActionList.Item key={f.key} selected={f.key === field} onSelect={() => setField(f.key)}>
                      {f.name}
                    </ActionList.Item>
                  ))}
                </ActionList>
              </ActionMenu.Overlay>
            </ActionMenu>
          </header>
          <div className="body">{d.field ? <Columns buckets={d.field.buckets} /> : <p className="muted">Pick a field.</p>}</div>
        </section>
      </div>
    </div>
  );
}

function BurnUp({ data }: { data: { status_options: { id: string | null; name: string; color: string }[]; burnup: { date: string; counts: number[] }[] } }) {
  const w = 900;
  const h = 260;
  const padL = 32;
  const padB = 24;
  const rows = data.burnup;
  const n = rows.length;
  const max = Math.max(1, ...rows.map((r) => r.counts.reduce((a, b) => a + b, 0)));
  const x = (i: number) => padL + (n <= 1 ? 0 : (i / (n - 1)) * (w - padL - 8));
  const y = (v: number) => h - padB - (v / max) * (h - padB - 12);
  // stacked from the bottom: No status first, then options in order
  const layers = data.status_options.map((_, li) => rows.map((r) => r.counts.slice(0, li + 1).reduce((a, b) => a + b, 0)));
  const ticks = [0, Math.ceil(max / 2), max];
  const labelEvery = Math.max(1, Math.floor(n / 6));
  return (
    <div>
      <svg viewBox={`0 0 ${w} ${h}`} className="chart burnup" role="img" aria-label="Items over time by status">
        {ticks.map((t) => (
          <g key={t}>
            <line x1={padL} x2={w - 8} y1={y(t)} y2={y(t)} stroke="var(--borderColor-default)" strokeDasharray="2 3" />
            <text x={padL - 6} y={y(t) + 4} textAnchor="end" className="contrib-label">
              {t}
            </text>
          </g>
        ))}
        {layers
          .map((top, li) => {
            const bottom = li === 0 ? rows.map(() => 0) : layers[li - 1];
            const path = [...top.map((v, i) => `${i === 0 ? 'M' : 'L'}${x(i)},${y(v)}`), ...bottom.map((v, i) => `L${x(n - 1 - i)},${y(bottom[n - 1 - i] ?? v)}`).slice(0), 'Z'].join(' ');
            return <path key={li} d={path} fill={colorOf(data.status_options[li].color)} fillOpacity={0.55} stroke={colorOf(data.status_options[li].color)} strokeWidth={1} />;
          })
          .reverse()}
        {rows.map((r, i) => (i % labelEvery === 0 || i === n - 1 ? <text key={r.date} x={x(i)} y={h - 6} textAnchor="middle" className="contrib-label">{r.date.slice(5)}</text> : null))}
      </svg>
      <div className="chart-legend">
        {data.status_options.map((o) => (
          <span key={o.name}>
            <span className="dot" style={{ background: colorOf(o.color) }} /> {o.name}
          </span>
        ))}
      </div>
    </div>
  );
}

function Columns({ buckets }: { buckets: { name: string; color: string; count: number }[] }) {
  const max = Math.max(1, ...buckets.map((b) => b.count));
  if (buckets.length === 0) return <p className="muted">No items.</p>;
  return (
    <div className="columns-chart">
      {buckets.slice(0, 12).map((b) => (
        <div key={b.name} className="columns-col" title={`${b.name}: ${b.count}`}>
          <span className="columns-value">{b.count}</span>
          <span className="columns-bar" style={{ height: `${(b.count / max) * 100}%`, background: colorOf(b.color) }} />
          <span className="columns-label">{b.name}</span>
        </div>
      ))}
    </div>
  );
}
