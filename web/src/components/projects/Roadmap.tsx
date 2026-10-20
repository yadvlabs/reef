// Roadmap layout: one row per item on a time axis, bars from a start date field to a target
// date field, draggable edges, a "Today" marker, and optional swimlanes.
import { ActionList, ActionMenu, Button } from '@primer/react';
import { CalendarIcon, ChevronDownIcon, ChevronRightIcon, TriangleDownIcon } from '@primer/octicons-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { itemTitle, OPTION_DOTS, type ProjectField, type ProjectFull, type ProjectItem, type ProjectView } from '../../lib/projects';
import { ContentIcon, useProjectMutations, viewItems } from '../ProjectBoard';

const DAY = 86_400_000;
const ZOOMS: Record<'month' | 'quarter' | 'year', { px: number; label: string }> = { month: { px: 36, label: 'Month' }, quarter: { px: 12, label: 'Quarter' }, year: { px: 4, label: 'Year' } };

const dayOf = (iso: string | null | undefined): number | null => {
  if (!iso) return null;
  const t = Date.parse(iso.length === 10 ? iso + 'T00:00:00Z' : iso);
  return Number.isNaN(t) ? null : Math.floor(t / DAY);
};
const isoOf = (day: number) => new Date(day * DAY).toISOString().slice(0, 10);
const monthLabel = (day: number) => new Date(day * DAY).toLocaleDateString(undefined, { month: 'short', year: 'numeric', timeZone: 'UTC' });

/// Ensure the view has usable date fields; returns them (or nulls while they are being created).
export function useRoadmapFields(project: ProjectFull, view: ProjectView): { start: ProjectField | null; target: ProjectField | null } {
  const m = useProjectMutations(project);
  const dates = project.fields.filter((f) => f.data_type === 'date');
  const s = view.settings;
  const start = dates.find((f) => f.id === s.start_field_id) ?? dates.find((f) => /start/i.test(f.name)) ?? null;
  const target = dates.find((f) => f.id === s.target_field_id) ?? dates.find((f) => /target|end|due/i.test(f.name)) ?? dates.find((f) => f.id !== start?.id) ?? null;
  const creating = useRef(false);
  useEffect(() => {
    if (!project.permissions.write || creating.current) return;
    if (!start || !target) {
      creating.current = true;
      const run = async () => {
        if (!start) await m.createField.mutateAsync({ name: 'Start date', data_type: 'date' });
        if (!target) await m.createField.mutateAsync({ name: 'Target date', data_type: 'date' });
        creating.current = false;
      };
      run().catch(() => (creating.current = false));
    } else if (s.start_field_id !== start.id || s.target_field_id !== target.id) {
      m.updateView.mutate({ view: view.id, patch: { settings: { start_field_id: start.id, target_field_id: target.id } } });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [start?.id, target?.id, project.permissions.write, view.id]);
  return { start, target };
}

interface Bucket {
  id: string | null;
  name: string;
  color?: string;
}

export function Roadmap({ project, view, filter, onOpen }: { project: ProjectFull; view: ProjectView; filter: string; onOpen: (item: ProjectItem) => void }) {
  const m = useProjectMutations(project);
  const canWrite = project.permissions.write;
  const { start, target } = useRoadmapFields(project, view);
  const zoom = view.settings.zoom ?? 'month';
  const px = ZOOMS[zoom].px;
  const items = viewItems(project, view, filter);
  const laneField = project.fields.find((f) => f.id === view.settings.group_by_field_id && f.data_type === 'single_select');
  const lanes: Bucket[] = laneField ? [...laneField.options.map((o) => ({ id: o.id, name: o.name, color: o.color })), { id: null, name: `No ${laneField.name}` }] : [{ id: null, name: '' }];
  const today = Math.floor(Date.now() / DAY);
  const dated = items.map((it) => ({ it, s: dayOf(start ? it.values[String(start.id)] : null), e: dayOf(target ? it.values[String(target.id)] : null) }));
  const allDays = dated.flatMap((d) => [d.s, d.e].filter((x): x is number => x !== null));
  const span = zoom === 'month' ? 45 : zoom === 'quarter' ? 120 : 400;
  const winStart = Math.min(today - Math.floor(span / 3), ...allDays.map((d) => d - 7));
  const winEnd = Math.max(today + span, ...allDays.map((d) => d + 14));
  const width = (winEnd - winStart + 1) * px;
  const scroller = useRef<HTMLDivElement>(null);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const scrollToToday = () => scroller.current?.scrollTo({ left: Math.max(0, (today - winStart) * px - scroller.current.clientWidth / 3), behavior: 'smooth' });
  useEffect(() => {
    scroller.current?.scrollTo({ left: Math.max(0, (today - winStart) * px - 200) });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [zoom, view.id]);
  const months = useMemo(() => {
    const out: { day: number; label: string; days: number }[] = [];
    let d = winStart;
    while (d <= winEnd) {
      const date = new Date(d * DAY);
      const next = Date.UTC(date.getUTCFullYear(), date.getUTCMonth() + 1, 1) / DAY;
      const end = Math.min(next - 1, winEnd);
      out.push({ day: d, label: monthLabel(d), days: end - d + 1 });
      d = end + 1;
    }
    return out;
  }, [winStart, winEnd]);
  const setDates = (it: ProjectItem, s: number | null, e: number | null) => {
    if (!start || !target) return;
    m.setValues.mutate({ item: it.id, values: { [String(start.id)]: s === null ? null : isoOf(s), [String(target.id)]: e === null ? null : isoOf(e) } });
  };
  const setZoom = (z: 'month' | 'quarter' | 'year') => m.updateView.mutate({ view: view.id, patch: { settings: { zoom: z } } });
  const rowsFor = (lane: Bucket) => (laneField ? dated.filter((d) => (d.it.values[String(laneField.id)] ?? null) === lane.id) : dated);
  return (
    <div className="roadmap">
      <div className="roadmap-toolbar">
        <Button size="small" leadingVisual={CalendarIcon} onClick={scrollToToday}>
          Today
        </Button>
        <ActionMenu>
          <ActionMenu.Button size="small" trailingAction={TriangleDownIcon}>
            {ZOOMS[zoom].label}
          </ActionMenu.Button>
          <ActionMenu.Overlay>
            <ActionList selectionVariant="single">
              {(Object.keys(ZOOMS) as (keyof typeof ZOOMS)[]).map((z) => (
                <ActionList.Item key={z} selected={z === zoom} onSelect={() => setZoom(z)}>
                  {ZOOMS[z].label}
                </ActionList.Item>
              ))}
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
        <span className="muted small">
          {start && target ? (
            <>
              Dates: <strong>{start.name}</strong> → <strong>{target.name}</strong>
            </>
          ) : (
            'Creating date fields…'
          )}
        </span>
      </div>
      <div className="roadmap-body">
        <div className="roadmap-titles">
          <div className="roadmap-head-cell">Title</div>
          {lanes.map((lane) => {
            const rows = rowsFor(lane);
            if (laneField && rows.length === 0 && lane.id === null) return null;
            const key = lane.id ?? '__none';
            const isCollapsed = !!collapsed[key];
            return (
              <div key={key}>
                {laneField ? (
                  <button className="roadmap-lane-head" onClick={() => setCollapsed({ ...collapsed, [key]: !isCollapsed })}>
                    {isCollapsed ? <ChevronRightIcon /> : <ChevronDownIcon />}
                    {lane.color ? <span className="dot" style={{ background: OPTION_DOTS[lane.color] }} /> : null}
                    <strong>{lane.name}</strong> <span className="count">{rows.length}</span>
                  </button>
                ) : null}
                {!isCollapsed
                  ? rows.map(({ it, s, e }) => (
                      <div key={it.id} className="roadmap-row title" onClick={() => onOpen(it)}>
                        <ContentIcon c={it.content} />
                        <span className="roadmap-title">{itemTitle(it)}</span>
                        {s === null && e === null ? <span className="muted small">No dates</span> : null}
                      </div>
                    ))
                  : null}
              </div>
            );
          })}
        </div>
        <div className="roadmap-scroll" ref={scroller}>
          <div className="roadmap-canvas" style={{ width }}>
            <div className="roadmap-head-cell months">
              {months.map((mo) => (
                <span key={mo.day} className="roadmap-month" style={{ left: (mo.day - winStart) * px, width: mo.days * px }}>
                  {mo.label}
                </span>
              ))}
            </div>
            {zoom === 'month' ? (
              <div className="roadmap-days">
                {Array.from({ length: winEnd - winStart + 1 }, (_, i) => winStart + i).map((d) => {
                  const dow = new Date(d * DAY).getUTCDay();
                  return <span key={d} className={`roadmap-day${dow === 0 || dow === 6 ? ' weekend' : ''}`} style={{ left: (d - winStart) * px, width: px }} />;
                })}
              </div>
            ) : null}
            <div className="roadmap-today" style={{ left: (today - winStart) * px }} title="Today" />
            {lanes.map((lane) => {
              const rows = rowsFor(lane);
              if (laneField && rows.length === 0 && lane.id === null) return null;
              const key = lane.id ?? '__none';
              const isCollapsed = !!collapsed[key];
              return (
                <div key={key}>
                  {laneField ? <div className="roadmap-lane-head spacer" /> : null}
                  {!isCollapsed ? rows.map(({ it, s, e }) => <Row key={it.id} item={it} s={s} e={e} px={px} winStart={winStart} canWrite={canWrite && !!start && !!target} onChange={(ns, ne) => setDates(it, ns, ne)} onOpen={() => onOpen(it)} />) : null}
                </div>
              );
            })}
          </div>
        </div>
      </div>
      {items.length === 0 ? <p className="muted" style={{ padding: 24 }}>No items match this view.</p> : null}
    </div>
  );
}

function Row({ item, s, e, px, winStart, canWrite, onChange, onOpen }: { item: ProjectItem; s: number | null; e: number | null; px: number; winStart: number; canWrite: boolean; onChange: (s: number | null, e: number | null) => void; onOpen: () => void }) {
  const [drag, setDrag] = useState<{ mode: 'move' | 'start' | 'end'; x0: number; s: number; e: number; cur: { s: number; e: number } } | null>(null);
  const startDay = s ?? e;
  const endDay = e ?? s;
  const shown = drag ? drag.cur : startDay !== null && endDay !== null ? { s: Math.min(startDay, endDay), e: Math.max(startDay, endDay) } : null;
  useEffect(() => {
    if (!drag) return;
    const move = (ev: MouseEvent) => {
      const delta = Math.round((ev.clientX - drag.x0) / px);
      let ns = drag.s;
      let ne = drag.e;
      if (drag.mode === 'move') {
        ns += delta;
        ne += delta;
      } else if (drag.mode === 'start') ns = Math.min(drag.s + delta, drag.e);
      else ne = Math.max(drag.e + delta, drag.s);
      setDrag({ ...drag, cur: { s: ns, e: ne } });
    };
    const up = () => {
      if (drag.cur.s !== drag.s || drag.cur.e !== drag.e) onChange(drag.cur.s, drag.cur.e);
      setDrag(null);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
    return () => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
    };
  }, [drag, px, onChange]);
  const begin = (mode: 'move' | 'start' | 'end') => (ev: React.MouseEvent) => {
    if (!canWrite || !shown) return;
    ev.preventDefault();
    ev.stopPropagation();
    setDrag({ mode, x0: ev.clientX, s: shown.s, e: shown.e, cur: shown });
  };
  const milestone = shown && shown.s === shown.e;
  return (
    <div
      className="roadmap-row"
      onClick={(ev) => {
        if (shown || !canWrite) return;
        const rect = (ev.currentTarget as HTMLElement).getBoundingClientRect();
        const day = winStart + Math.floor((ev.clientX - rect.left) / px);
        onChange(day, day + 6);
      }}
      title={!shown && canWrite ? 'Click to set dates' : undefined}
    >
      {shown ? (
        <div
          className={`roadmap-bar${milestone ? ' milestone' : ''}${drag ? ' dragging' : ''}`}
          style={{ left: (shown.s - winStart) * px, width: Math.max(px, (shown.e - shown.s + 1) * px) }}
          onMouseDown={begin('move')}
          onClick={(ev) => {
            ev.stopPropagation();
            if (!drag) onOpen();
          }}
          title={`${isoOf(shown.s)} → ${isoOf(shown.e)}`}
        >
          {canWrite && !milestone ? <span className="handle left" onMouseDown={begin('start')} /> : null}
          <span className="bar-label">{itemTitle(item)}</span>
          {canWrite && !milestone ? <span className="handle right" onMouseDown={begin('end')} /> : null}
        </div>
      ) : null}
    </div>
  );
}
