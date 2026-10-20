// Project views: board (with optional swimlanes), table (with grouping), the item side panel,
// and the shared editors for field values.
import { ActionList, ActionMenu, Avatar, Button, Dialog, FormControl, IconButton, Label, TextInput, Textarea } from '@primer/react';
import { IssueOpenedIcon, IssueClosedIcon, GitPullRequestIcon, GitMergeIcon, NoteIcon, PlusIcon, KebabHorizontalIcon, XIcon, ChevronDownIcon, ChevronRightIcon, LinkExternalIcon, TrashIcon, ArchiveIcon, IssueDraftIcon, SkipIcon, TriangleDownIcon, PencilIcon, PaintbrushIcon, ArrowLeftIcon, ArrowRightIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState, useRef, useEffect } from 'react';
import { labelTextColor } from '../lib/issues';
import { reef } from '../lib/api';
import { BUILTIN_FIELDS, OPTION_COLORS, OPTION_DOTS, itemTitle, parseFilter, sortItems, type ProjectField, type ProjectFull, type ProjectItem, type ProjectView, type ViewSettings } from '../lib/projects';
import { Markdown } from './Markdown';
import { RelativeTime } from '@primer/react';
import { api, type User } from '../lib/api';
import { useSession } from '../lib/session';

export function ContentIcon({ c }: { c: ProjectItem['content'] }) {
  if (c.type === 'draft') return <IssueDraftIcon className="muted" />;
  if (c.type === 'pull') return c.merged ? <GitMergeIcon fill="var(--fgColor-done)" /> : c.state === 'closed' ? <GitPullRequestIcon fill="var(--fgColor-danger)" /> : <GitPullRequestIcon fill="var(--fgColor-success)" />;
  if (c.type === 'issue') return c.state === 'closed' ? (c.state_reason === 'not_planned' ? <SkipIcon className="muted" /> : <IssueClosedIcon fill="var(--fgColor-done)" />) : <IssueOpenedIcon fill="var(--fgColor-success)" />;
  return <NoteIcon className="muted" />;
}

export function useProjectMutations(project: ProjectFull) {
  const qc = useQueryClient();
  const base = `/projects/${project.owner.login}/${project.number}`;
  const key = ['project', project.owner.login.toLowerCase(), project.number];
  const invalidate = () => qc.invalidateQueries({ queryKey: key });
  const setValues = useMutation({
    mutationFn: ({ item, values }: { item: number; values: Record<string, string | null> }) => reef(`${base}/items/${item}`, { method: 'PATCH', json: { values } }),
    onMutate: async ({ item, values }) => {
      await qc.cancelQueries({ queryKey: key });
      const prev = qc.getQueryData<ProjectFull>(key);
      if (prev) qc.setQueryData<ProjectFull>(key, { ...prev, items: prev.items.map((i) => (i.id === item ? { ...i, values: { ...i.values, ...values } } : i)) });
      return { prev };
    },
    onError: (_e, _v, ctx) => ctx?.prev && qc.setQueryData(key, ctx.prev),
    onSettled: invalidate,
  });
  const move = useMutation({ mutationFn: ({ item, after }: { item: number; after: number | null }) => reef(`${base}/items/${item}/move`, { method: 'POST', json: { after } }), onSettled: invalidate });
  const add = useMutation({ mutationFn: (body: { issue?: string; title?: string; values?: Record<string, string> }) => reef<ProjectItem>(`${base}/items`, { method: 'POST', json: body }), onSettled: invalidate });
  const remove = useMutation({ mutationFn: (item: number) => reef(`${base}/items/${item}`, { method: 'DELETE' }), onSettled: invalidate });
  const archive = useMutation({ mutationFn: ({ item, archived }: { item: number; archived: boolean }) => reef(`${base}/items/${item}`, { method: 'PATCH', json: { archived } }), onSettled: invalidate });
  const updateDraft = useMutation({ mutationFn: ({ item, title, body }: { item: number; title?: string; body?: string }) => reef(`${base}/items/${item}`, { method: 'PATCH', json: { title, body } }), onSettled: invalidate });
  const convert = useMutation({ mutationFn: ({ item, repo }: { item: number; repo: string }) => reef<ProjectItem>(`${base}/items/${item}/convert`, { method: 'POST', json: { repo } }), onSettled: invalidate });
  const updateView = useMutation({ mutationFn: ({ view, patch }: { view: number; patch: { name?: string; layout?: string; settings?: Partial<ViewSettings> } }) => reef(`${base}/views/${view}`, { method: 'PATCH', json: patch }), onSettled: invalidate });
  const createView = useMutation({ mutationFn: (v: { name: string; layout: string; settings: ViewSettings }) => reef<ProjectView>(`${base}/views`, { method: 'POST', json: v }), onSettled: invalidate });
  const deleteView = useMutation({ mutationFn: (view: number) => reef(`${base}/views/${view}`, { method: 'DELETE' }), onSettled: invalidate });
  const createField = useMutation({ mutationFn: (v: { name: string; data_type: string; options?: { name: string; color: string; description?: string }[] }) => reef<ProjectField>(`${base}/fields`, { method: 'POST', json: v }), onSettled: invalidate });
  const updateField = useMutation({ mutationFn: (v: { id: number; name?: string; options?: { id?: string; name: string; color: string; description: string }[] }) => reef(`${base}/fields/${v.id}`, { method: 'PATCH', json: { name: v.name, options: v.options } }), onSettled: invalidate });
  return { setValues, move, add, remove, archive, updateDraft, convert, updateView, createView, deleteView, createField, updateField, invalidate, base };
}

/// Items visible in a view after its filter and sort.
export function viewItems(project: ProjectFull, view: ProjectView, extraFilter: string): ProjectItem[] {
  const q = [view.settings.filter ?? '', extraFilter].filter(Boolean).join(' ');
  const pred = parseFilter(q);
  const items = project.items.filter((i) => !i.archived && pred(i, project.fields));
  return sortItems(items, project.fields, view.settings.sort ?? null);
}

function OptionDot({ color }: { color?: string }) {
  return <span className="dot" style={{ background: color ? OPTION_DOTS[color] ?? 'var(--fgColor-muted)' : 'var(--fgColor-muted)' }} />;
}

/// A single-select value as a coloured pill.
export function OptionPill({ field, value }: { field: ProjectField; value: string | null | undefined }) {
  const o = field.options.find((x) => x.id === value);
  if (!o) return <span className="muted">—</span>;
  return (
    <span className="option-pill" style={{ background: OPTION_COLORS[o.color] ?? OPTION_COLORS.GRAY }}>
      <OptionDot color={o.color} /> {o.name}
    </span>
  );
}

/// Editor for one field value, used by table cells and the side panel.
export function FieldValueEditor({ project, item, field, canWrite, compact }: { project: ProjectFull; item: ProjectItem; field: ProjectField; canWrite: boolean; compact?: boolean }) {
  const m = useProjectMutations(project);
  const v = item.values[String(field.id)] ?? null;
  if (field.data_type === 'single_select') {
    return (
      <ActionMenu>
        <ActionMenu.Anchor>
          <button type="button" className={`cell-button select-button${compact ? ' compact' : ''}`} disabled={!canWrite} aria-label={field.name} title={canWrite ? `Change ${field.name}` : `${field.name} (read-only)`}>
            <OptionPill field={field} value={v} />
            {canWrite ? <TriangleDownIcon size={12} className="caret" /> : null}
          </button>
        </ActionMenu.Anchor>
        <ActionMenu.Overlay>
          <ActionList selectionVariant="single">
            <ActionList.Item selected={!v} onSelect={() => m.setValues.mutate({ item: item.id, values: { [String(field.id)]: null } })}>
              <span className="muted">Clear</span>
            </ActionList.Item>
            {field.options.map((o) => (
              <ActionList.Item key={o.id} selected={v === o.id} onSelect={() => m.setValues.mutate({ item: item.id, values: { [String(field.id)]: o.id } })}>
                <ActionList.LeadingVisual>
                  <OptionDot color={o.color} />
                </ActionList.LeadingVisual>
                {o.name}
                {o.description ? <ActionList.Description variant="block">{o.description}</ActionList.Description> : null}
              </ActionList.Item>
            ))}
          </ActionList>
        </ActionMenu.Overlay>
      </ActionMenu>
    );
  }
  return (
    <input
      className="cell-input"
      type={field.data_type === 'number' ? 'number' : field.data_type === 'date' ? 'date' : 'text'}
      defaultValue={v ?? ''}
      disabled={!canWrite}
      aria-label={field.name}
      onBlur={(e) => {
        const nv = e.target.value;
        if (nv !== (v ?? '')) m.setValues.mutate({ item: item.id, values: { [String(field.id)]: nv || null } });
      }}
      onKeyDown={(e) => {
        if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
      }}
    />
  );
}

function BuiltinValue({ item, k }: { item: ProjectItem; k: string }) {
  const c = item.content;
  if (k === 'assignees') return <span className="avatars">{c.assignees?.map((a) => <Avatar key={a.id} src={a.avatar_url} size={20} alt={a.login} />)}</span>;
  if (k === 'labels')
    return (
      <span className="card-labels" style={{ marginTop: 0 }}>
        {c.labels?.map((l) => (
          <span key={l.name} className="mini-label" style={{ borderColor: `#${l.color}`, color: `#${l.color}` }}>
            {l.name}
          </span>
        ))}
      </span>
    );
  if (k === 'repository') return <span className="muted small">{c.repository ?? ''}</span>;
  if (k === 'milestone') return <span className="small">{c.milestone ?? ''}</span>;
  return null;
}

export function Card({ item, project, view, onOpen, canWrite }: { item: ProjectItem; project: ProjectFull; view: ProjectView; onOpen: () => void; canWrite: boolean }) {
  const c = item.content;
  const m = useProjectMutations(project);
  const visible = view.settings.visible_fields ?? ['assignees', 'labels'];
  const customShown = project.fields.filter((f) => visible.includes(String(f.id)) && f.id !== view.settings.column_field_id && f.id !== view.settings.group_by_field_id && (canWrite || item.values[String(f.id)]));
  return (
    <div
      className="project-card"
      draggable={canWrite}
      onDragStart={(e) => {
        e.dataTransfer.setData('text/reef-item', String(item.id));
        e.dataTransfer.effectAllowed = 'move';
      }}
      onClick={onOpen}
    >
      <div className="card-meta">
        <ContentIcon c={c} />
        <span className="muted small">{c.type === 'draft' ? 'Draft' : c.repository ? `${c.repository.split('/')[1]} #${c.number}` : ''}</span>
        {canWrite ? (
          <span style={{ marginLeft: 'auto' }} onClick={(e) => e.stopPropagation()}>
            <ActionMenu>
              <ActionMenu.Anchor>
                <button className="linkish muted card-menu" aria-label="Item options">
                  <KebabHorizontalIcon />
                </button>
              </ActionMenu.Anchor>
              <ActionMenu.Overlay align="end">
                <ActionList>
                  {c.url ? (
                    <ActionList.LinkItem href={c.url} target="_blank">
                      <ActionList.LeadingVisual>
                        <LinkExternalIcon />
                      </ActionList.LeadingVisual>
                      Open in new tab
                    </ActionList.LinkItem>
                  ) : null}
                  <ActionList.Item onSelect={() => m.archive.mutate({ item: item.id, archived: true })}>
                    <ActionList.LeadingVisual>
                      <ArchiveIcon />
                    </ActionList.LeadingVisual>
                    Archive
                  </ActionList.Item>
                  <ActionList.Item variant="danger" onSelect={() => confirm('Remove this item from the project?') && m.remove.mutate(item.id)}>
                    <ActionList.LeadingVisual>
                      <TrashIcon />
                    </ActionList.LeadingVisual>
                    Delete from project
                  </ActionList.Item>
                </ActionList>
              </ActionMenu.Overlay>
            </ActionMenu>
          </span>
        ) : null}
      </div>
      <div className="card-title">{itemTitle(item)}</div>
      {visible.includes('labels') && c.labels?.length ? (
        <div className="card-labels">
          {c.labels.map((l) => (
            <span key={l.name} className="mini-label" style={{ background: `#${l.color}`, color: labelTextColor(l.color) }}>
              {l.name}
            </span>
          ))}
        </div>
      ) : null}
      {customShown.length ? (
        <div className="card-fields" onClick={(e) => e.stopPropagation()}>
          {customShown.map((f) => (
            <span key={f.id} className="card-field">
              <span className="muted">{f.name}</span> <FieldValueEditor project={project} item={item} field={f} canWrite={canWrite} compact />
            </span>
          ))}
        </div>
      ) : null}
      {visible.includes('assignees') && c.assignees?.length ? (
        <div className="card-assignees">
          {c.assignees.map((a) => (
            <Avatar key={a.id} src={a.avatar_url} size={20} alt={a.login} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

export function AddItemBox({ project, values, autoOpen }: { project: ProjectFull; values?: Record<string, string>; autoOpen?: boolean }) {
  const m = useProjectMutations(project);
  const [open, setOpen] = useState(!!autoOpen);
  const [text, setText] = useState('');
  const search = useQuery({ queryKey: ['project-search', project.id, text], queryFn: () => reef<{ id: number; number: number; title: string; repository: string; is_pull: boolean; added: boolean }[]>(`${m.base}/search?q=${encodeURIComponent(text)}`), enabled: open && text.length > 0 && !text.startsWith('#') });
  if (!open) {
    return (
      <button className="add-item" onClick={() => setOpen(true)}>
        <PlusIcon /> Add item
      </button>
    );
  }
  const submit = async () => {
    const t = text.trim();
    if (!t) return;
    const ref = t.match(/^(?:([\w.-]+\/[\w.-]+))?#(\d+)$/);
    await m.add.mutateAsync(ref ? { issue: `${ref[1] ?? ''}#${ref[2]}`, values } : { title: t, values });
    setText('');
  };
  return (
    <div className="add-item-box">
      <TextInput
        block
        size="small"
        autoFocus
        placeholder="Type to create a draft, or #123 / owner/repo#123"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') submit();
          if (e.key === 'Escape') setOpen(false);
        }}
      />
      {search.data?.length ? (
        <div className="add-item-results">
          {search.data.slice(0, 8).map((r) => (
            <button
              key={r.id}
              className="result"
              disabled={r.added}
              onClick={async () => {
                await m.add.mutateAsync({ issue: `${r.repository}#${r.number}`, values });
                setText('');
              }}
            >
              {r.is_pull ? <GitPullRequestIcon /> : <IssueOpenedIcon />} <span className="muted small">{r.repository}#{r.number}</span> {r.title} {r.added ? <span className="muted small">(added)</span> : null}
            </button>
          ))}
        </div>
      ) : null}
      <div style={{ display: 'flex', gap: 4, marginTop: 4 }}>
        <Button size="small" variant="primary" onClick={submit} disabled={!text.trim()}>
          Add
        </Button>
        <Button size="small" variant="invisible" onClick={() => setOpen(false)} leadingVisual={XIcon}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

type Bucket = { id: string | null; name: string; color?: string };
function buckets(field: ProjectField | undefined, fallback: string): Bucket[] {
  return field ? [...field.options.map((o) => ({ id: o.id, name: o.name, color: o.color })), { id: null, name: `No ${field.name}` }] : [{ id: null, name: fallback }];
}

export function Board({ project, view, filter, onOpen }: { project: ProjectFull; view: ProjectView; filter: string; onOpen: (item: ProjectItem) => void }) {
  const m = useProjectMutations(project);
  const canWrite = project.permissions.write;
  const s = view.settings;
  const colField = project.fields.find((f) => f.id === s.column_field_id) ?? project.fields.find((f) => f.data_type === 'single_select');
  const laneField = project.fields.find((f) => f.id === s.group_by_field_id && f.id !== colField?.id);
  const columns = buckets(colField, 'Items');
  const lanes: Bucket[] = laneField ? buckets(laneField, '') : [{ id: undefined as unknown as null, name: '' }];
  const items = viewItems(project, view, filter);
  const [dragOver, setDragOver] = useState<{ key: string; index: number } | null>(null);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [addingColumn, setAddingColumn] = useState(false);
  const [newColumn, setNewColumn] = useState('');
  const addColumn = () => {
    const name = newColumn.trim();
    if (!colField || !name) return;
    m.updateField.mutate({ id: colField.id, options: [...colField.options, { name, color: ['GRAY', 'BLUE', 'GREEN', 'YELLOW', 'ORANGE', 'RED', 'PINK', 'PURPLE'][colField.options.length % 8], description: '' }] });
    setNewColumn('');
    setAddingColumn(false);
  };
  /// Where a dragged card would land: the index of the first card whose middle is below the pointer.
  const indexAt = (e: React.DragEvent, container: HTMLElement): number => {
    // Cards sit inside `display: contents` wrappers, so look one level down as well.
    const cards = Array.from(container.querySelectorAll<HTMLElement>(':scope > .project-card, :scope > * > .project-card'));
    let i = 0;
    for (const c of cards) {
      const r = c.getBoundingClientRect();
      if (e.clientY < r.top + r.height / 2) break;
      i++;
    }
    return i;
  };
  const valueOf = (it: ProjectItem, f?: ProjectField) => (f ? (it.values[String(f.id)] ?? null) : null);
  const inCell = (col: Bucket, lane: Bucket) => items.filter((i) => valueOf(i, colField) === col.id && (!laneField || valueOf(i, laneField) === lane.id));
  return (
    <div className="board-scroll">
      <div className="board-grid" style={{ gridTemplateColumns: `repeat(${columns.length}, minmax(300px, 1fr))` }}>
        {!laneField ? (
          <div className="board-colheads" style={{ display: 'contents' }}>
            {columns.map((col) => (
              <ColumnHead key={col.id ?? '__none'} col={col} count={items.filter((i) => valueOf(i, colField) === col.id).length} field={colField} project={project} canWrite={canWrite} />
            ))}
          </div>
        ) : null}
        {lanes.map((lane) => {
          const laneKey = lane.id ?? '__none';
          const laneCount = laneField ? items.filter((i) => valueOf(i, laneField) === lane.id).length : items.length;
          const isCollapsed = !!collapsed[laneKey];
          return (
            <div key={laneKey} style={{ display: 'contents' }}>
              {laneField ? (
                <div className="lane-head" style={{ gridColumn: '1 / -1' }}>
                  <button className="linkish" onClick={() => setCollapsed({ ...collapsed, [laneKey]: !isCollapsed })} aria-label={isCollapsed ? 'Expand' : 'Collapse'}>
                    {isCollapsed ? <ChevronRightIcon /> : <ChevronDownIcon />}
                  </button>
                  <OptionDot color={lane.color} />
                  <strong>{lane.name}</strong>
                  <span className="count">{laneCount}</span>
                  <span className="muted small">{laneField.name}</span>
                </div>
              ) : null}
              {!isCollapsed
                ? columns.map((col) => {
                    const cell = inCell(col, lane);
                    const key = `${laneKey}:${col.id ?? '__none'}`;
                    const overIndex = dragOver?.key === key ? dragOver.index : -1;
                    const seed: Record<string, string> = {};
                    if (colField && col.id) seed[String(colField.id)] = col.id;
                    if (laneField && lane.id) seed[String(laneField.id)] = lane.id;
                    return (
                      <div
                        key={key}
                        className={`column${overIndex >= 0 ? ' over' : ''}${laneField ? ' in-lane' : ''}`}
                        onDragOver={(e) => {
                          if (!canWrite) return;
                          e.preventDefault();
                          const body = (e.currentTarget as HTMLElement).querySelector<HTMLElement>('.column-body');
                          const idx = body ? indexAt(e, body) : cell.length;
                          if (dragOver?.key !== key || dragOver.index !== idx) setDragOver({ key, index: idx });
                        }}
                        onDragLeave={(e) => {
                          if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node)) setDragOver(null);
                        }}
                        onDrop={(e) => {
                          e.preventDefault();
                          let idx = dragOver?.key === key ? dragOver.index : cell.length;
                          setDragOver(null);
                          const id = Number(e.dataTransfer.getData('text/reef-item'));
                          const item = project.items.find((i) => i.id === id);
                          if (!id || !item) return;
                          // The pointer index counts the dragged card itself when it comes from this cell.
                          const from = cell.findIndex((i) => i.id === id);
                          if (from >= 0 && from < idx) idx -= 1;
                          if (from === idx && !laneField && colField && valueOf(item, colField) === col.id) return;
                          const values: Record<string, string | null> = {};
                          if (colField && valueOf(item, colField) !== col.id) values[String(colField.id)] = col.id;
                          if (laneField && valueOf(item, laneField) !== lane.id) values[String(laneField.id)] = lane.id;
                          if (Object.keys(values).length) m.setValues.mutate({ item: id, values });
                          const others = cell.filter((i) => i.id !== id);
                          const before = others.slice(0, idx).at(-1);
                          m.move.mutate({ item: id, after: before ? before.id : null });
                        }}
                      >
                        {laneField ? <ColumnHead col={col} count={cell.length} small field={colField} project={project} canWrite={canWrite} /> : null}
                        <div className="column-body">
                          {cell.map((it, i) => (
                            <div key={it.id} style={{ display: 'contents' }}>
                              {overIndex === i ? <div className="drop-line" /> : null}
                              <Card item={it} project={project} view={view} onOpen={() => onOpen(it)} canWrite={canWrite} />
                            </div>
                          ))}
                          {overIndex >= cell.length ? <div className="drop-line" /> : null}
                          {canWrite ? <AddItemBox project={project} values={Object.keys(seed).length ? seed : undefined} /> : null}
                        </div>
                      </div>
                    );
                  })
                : null}
            </div>
          );
        })}
      </div>
      {canWrite && colField ? (
        <div className={`add-column-strip${addingColumn ? ' open' : ''}`}>
          {addingColumn ? (
              <div className="add-item-box">
                <TextInput
                  block
                  size="small"
                  autoFocus
                  placeholder={`New ${colField.name.toLowerCase()} option`}
                  value={newColumn}
                  onChange={(e) => setNewColumn(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') addColumn();
                    if (e.key === 'Escape') setAddingColumn(false);
                  }}
                />
                <div style={{ display: 'flex', gap: 4, marginTop: 4 }}>
                  <Button size="small" variant="primary" onClick={addColumn} disabled={!newColumn.trim()}>
                    Add
                  </Button>
                  <Button size="small" variant="invisible" onClick={() => setAddingColumn(false)}>
                    Cancel
                  </Button>
                </div>
              </div>
            ) : (
            <IconButton icon={PlusIcon} aria-label={`Add ${colField.name.toLowerCase()} column`} title="Add column" variant="invisible" onClick={() => setAddingColumn(true)} />
          )}
        </div>
      ) : null}
      {!canWrite ? <p className="muted small" style={{ padding: '8px 0' }}>You have read-only access to this project.</p> : null}
    </div>
  );
}

const COLOR_NAMES = ['GRAY', 'BLUE', 'GREEN', 'YELLOW', 'ORANGE', 'RED', 'PINK', 'PURPLE'];

/// A board column header: option name, count, and (for writers) a menu to rename, recolour,
/// or delete the option behind it.
function ColumnHead({ col, count, small, field, project, canWrite }: { col: Bucket; count: number; small?: boolean; field?: ProjectField; project: ProjectFull; canWrite: boolean }) {
  const m = useProjectMutations(project);
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState(col.name);
  const option = field?.options.find((o) => o.id === col.id);
  const save = (patch: Partial<{ name: string; color: string }>) => {
    if (!field || !option) return;
    m.updateField.mutate({ id: field.id, options: field.options.map((o) => (o.id === option.id ? { ...o, ...patch } : o)) });
  };
  const index = field && option ? field.options.findIndex((o) => o.id === option.id) : -1;
  const move = (to: number) => {
    if (!field || !option || to < 0 || to >= field.options.length || to === index) return;
    const next = [...field.options];
    next.splice(index, 1);
    next.splice(to, 0, option);
    m.updateField.mutate({ id: field.id, options: next });
  };
  const commitRename = () => {
    const n = name.trim();
    if (n && n !== col.name) save({ name: n });
    setRenaming(false);
  };
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    // The action menu returns focus to its anchor after closing; claim it back for the input.
    if (!renaming) return;
    const t = setTimeout(() => inputRef.current?.focus(), 60);
    return () => clearTimeout(t);
  }, [renaming]);
  const draggable = canWrite && !!option && !!field;
  return (
    <div
      className={`column-head${small ? ' small' : ''}${draggable ? ' draggable' : ''}`}
      draggable={draggable && !renaming}
      onDragStart={(e) => {
        if (!draggable) return;
        e.dataTransfer.setData('text/reef-column', String(option!.id));
        e.dataTransfer.effectAllowed = 'move';
      }}
      onDragOver={(e) => {
        if (draggable && e.dataTransfer.types.includes('text/reef-column')) e.preventDefault();
      }}
      onDrop={(e) => {
        if (!draggable || !field) return;
        const id = e.dataTransfer.getData('text/reef-column');
        if (!id || id === String(option!.id)) return;
        e.preventDefault();
        e.stopPropagation();
        const from = field.options.findIndex((o) => String(o.id) === id);
        if (from < 0) return;
        const next = [...field.options];
        const [moved] = next.splice(from, 1);
        next.splice(index, 0, moved);
        m.updateField.mutate({ id: field.id, options: next });
      }}
    >
      <OptionDot color={col.color} />
      {renaming ? (
        <form
          style={{ flex: 1 }}
          onSubmit={(e) => {
            e.preventDefault();
            commitRename();
          }}
        >
          <TextInput
            ref={inputRef}
            size="small"
            block
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={commitRename}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                setName(col.name);
                setRenaming(false);
              }
            }}
            aria-label="Column name"
          />
        </form>
      ) : (
        <strong onDoubleClick={() => canWrite && option && setRenaming(true)} title={canWrite ? 'Double-click to rename' : undefined}>
          {col.name}
        </strong>
      )}
      <span className="count">{count}</span>
      {option?.description ? <span className="muted small col-desc" title={option.description}>{option.description}</span> : null}
      {canWrite && option && field ? (
        <span style={{ marginLeft: 'auto' }}>
          <ActionMenu>
            <ActionMenu.Anchor>
              <button className="linkish muted" aria-label="Column options">
                <KebabHorizontalIcon />
              </button>
            </ActionMenu.Anchor>
            <ActionMenu.Overlay align="end">
              <ActionList>
                <ActionList.Item
                  onSelect={() => {
                    setName(col.name);
                    setRenaming(true);
                  }}
                >
                  <ActionList.LeadingVisual>
                    <PencilIcon />
                  </ActionList.LeadingVisual>
                  Rename column
                </ActionList.Item>
                <ActionList.Item disabled={index <= 0} onSelect={() => move(index - 1)}>
                  <ActionList.LeadingVisual>
                    <ArrowLeftIcon />
                  </ActionList.LeadingVisual>
                  Move left
                </ActionList.Item>
                <ActionList.Item disabled={!field || index >= field.options.length - 1} onSelect={() => move(index + 1)}>
                  <ActionList.LeadingVisual>
                    <ArrowRightIcon />
                  </ActionList.LeadingVisual>
                  Move right
                </ActionList.Item>
                <ActionList.Group selectionVariant="single">
                  <ActionList.GroupHeading>
                    <PaintbrushIcon /> Colour
                  </ActionList.GroupHeading>
                  {COLOR_NAMES.map((c) => (
                    <ActionList.Item key={c} selected={option.color === c} onSelect={() => save({ color: c })}>
                      <ActionList.LeadingVisual>
                        <OptionDot color={c} />
                      </ActionList.LeadingVisual>
                      {c.charAt(0) + c.slice(1).toLowerCase()}
                    </ActionList.Item>
                  ))}
                </ActionList.Group>
                <ActionList.Divider />
                <ActionList.Item variant="danger" onSelect={() => confirm(`Delete the "${col.name}" option? Items in this column keep their other fields but lose this value.`) && m.updateField.mutate({ id: field.id, options: field.options.filter((o) => o.id !== option.id) })}>
                  <ActionList.LeadingVisual>
                    <TrashIcon />
                  </ActionList.LeadingVisual>
                  Delete column
                </ActionList.Item>
              </ActionList>
            </ActionMenu.Overlay>
          </ActionMenu>
        </span>
      ) : null}
    </div>
  );
}

export function Table({ project, view, filter, onOpen, onAddField }: { project: ProjectFull; view: ProjectView; filter: string; onOpen: (item: ProjectItem) => void; onAddField: () => void }) {
  const canWrite = project.permissions.write;
  const m = useProjectMutations(project);
  const s = view.settings;
  const visible = s.visible_fields ?? [...project.fields.map((f) => String(f.id)), 'assignees', 'labels', 'repository'];
  const columns: { key: string; name: string; field?: ProjectField }[] = visible
    .map((k) => {
      const f = project.fields.find((x) => String(x.id) === k);
      if (f) return { key: k, name: f.name, field: f };
      const b = BUILTIN_FIELDS.find((x) => x.key === k);
      return b ? { key: k, name: b.name } : null;
    })
    .filter((x): x is { key: string; name: string; field?: ProjectField } => !!x);
  const items = viewItems(project, view, filter);
  const groupField = project.fields.find((f) => f.id === s.group_by_field_id);
  const groups: { bucket: Bucket; items: ProjectItem[] }[] = groupField ? buckets(groupField, '').map((b) => ({ bucket: b, items: items.filter((i) => (i.values[String(groupField.id)] ?? null) === b.id) })) : [{ bucket: { id: null, name: '' }, items }];
  const setSort = (key: string) => {
    const cur = s.sort;
    const next = cur?.field === key ? (cur.direction === 'asc' ? { field: key, direction: 'desc' as const } : null) : { field: key, direction: 'asc' as const };
    m.updateView.mutate({ view: view.id, patch: { settings: { sort: next } } });
  };
  const hide = (key: string) => m.updateView.mutate({ view: view.id, patch: { settings: { visible_fields: visible.filter((k) => k !== key) } } });
  let n = 0;
  return (
    <div className="project-table-wrap">
      <table className="project-table">
        <thead>
          <tr>
            <th className="num">#</th>
            <th>
              <HeaderMenu name="Title" sortKey="title" sort={s.sort ?? null} onSort={setSort} canWrite={canWrite} />
            </th>
            {columns.map((c) => (
              <th key={c.key}>
                <HeaderMenu name={c.name} sortKey={c.key} sort={s.sort ?? null} onSort={setSort} onHide={() => hide(c.key)} canWrite={canWrite} />
              </th>
            ))}
            <th className="add-col">
              {canWrite ? (
                <ActionMenu>
                  <ActionMenu.Anchor>
                    <button className="linkish" aria-label="Add or show fields">
                      <PlusIcon />
                    </button>
                  </ActionMenu.Anchor>
                  <ActionMenu.Overlay align="end">
                    <ActionList>
                      <ActionList.Group>
                        <ActionList.GroupHeading>Hidden fields</ActionList.GroupHeading>
                        {[...project.fields.map((f) => ({ key: String(f.id), name: f.name })), ...BUILTIN_FIELDS]
                          .filter((f) => !visible.includes(f.key))
                          .map((f) => (
                            <ActionList.Item key={f.key} onSelect={() => m.updateView.mutate({ view: view.id, patch: { settings: { visible_fields: [...visible, f.key] } } })}>
                              {f.name}
                            </ActionList.Item>
                          ))}
                      </ActionList.Group>
                      <ActionList.Divider />
                      <ActionList.Item onSelect={onAddField}>
                        <ActionList.LeadingVisual>
                          <PlusIcon />
                        </ActionList.LeadingVisual>
                        New field
                      </ActionList.Item>
                    </ActionList>
                  </ActionMenu.Overlay>
                </ActionMenu>
              ) : null}
            </th>
          </tr>
        </thead>
        <tbody>
          {groups.map((g) => (
            <GroupRows key={g.bucket.id ?? '__none'} group={g} groupField={groupField} columns={columns} project={project} canWrite={canWrite} onOpen={onOpen} startIndex={(() => { const st = n; n += g.items.length; return st; })()} />
          ))}
        </tbody>
      </table>
      {canWrite && !groupField ? (
        <div style={{ padding: 8 }}>
          <AddItemBox project={project} />
        </div>
      ) : null}
    </div>
  );
}

function GroupRows({ group, groupField, columns, project, canWrite, onOpen, startIndex }: { group: { bucket: Bucket; items: ProjectItem[] }; groupField?: ProjectField; columns: { key: string; name: string; field?: ProjectField }[]; project: ProjectFull; canWrite: boolean; onOpen: (item: ProjectItem) => void; startIndex: number }) {
  const [open, setOpen] = useState(true);
  return (
    <>
      {groupField ? (
        <tr className="group-row">
          <td colSpan={columns.length + 3}>
            <button className="linkish" onClick={() => setOpen(!open)} aria-label={open ? 'Collapse' : 'Expand'}>
              {open ? <ChevronDownIcon /> : <ChevronRightIcon />}
            </button>
            <OptionDot color={group.bucket.color} /> <strong>{group.bucket.name}</strong> <span className="count">{group.items.length}</span>
          </td>
        </tr>
      ) : null}
      {open
        ? group.items.map((it, idx) => (
            <tr key={it.id}>
              <td className="muted num">{startIndex + idx + 1}</td>
              <td className="title-cell" onClick={() => onOpen(it)}>
                <span style={{ display: 'inline-flex', gap: 6, alignItems: 'center' }}>
                  <ContentIcon c={it.content} /> <span className="title-text">{itemTitle(it)}</span>
                  {it.content.number ? <span className="muted small">#{it.content.number}</span> : null}
                </span>
              </td>
              {columns.map((c) => (
                <td key={c.key}>{c.field ? <FieldValueEditor project={project} item={it} field={c.field} canWrite={canWrite} compact /> : <BuiltinValue item={it} k={c.key} />}</td>
              ))}
              <td />
            </tr>
          ))
        : null}
      {open && groupField && canWrite ? (
        <tr className="add-row">
          <td colSpan={columns.length + 3}>
            <AddItemBox project={project} values={group.bucket.id ? { [String(groupField.id)]: group.bucket.id } : undefined} />
          </td>
        </tr>
      ) : null}
    </>
  );
}

function HeaderMenu({ name, sortKey, sort, onSort, onHide, canWrite }: { name: string; sortKey: string; sort: ViewSettings['sort']; onSort: (k: string) => void; onHide?: () => void; canWrite: boolean }) {
  const active = sort?.field === sortKey;
  return (
    <ActionMenu>
      <ActionMenu.Anchor>
        <button className="th-button">
          {name} {active ? <span className="muted">{sort?.direction === 'asc' ? '↑' : '↓'}</span> : null}
        </button>
      </ActionMenu.Anchor>
      <ActionMenu.Overlay>
        <ActionList>
          <ActionList.Item onSelect={() => onSort(sortKey)}>{active && sort?.direction === 'asc' ? 'Sort descending' : 'Sort ascending'}</ActionList.Item>
          {active ? <ActionList.Item onSelect={() => onSort(sortKey)}>{sort?.direction === 'desc' ? 'Remove sort' : 'Sort descending'}</ActionList.Item> : null}
          {onHide && canWrite ? <ActionList.Item onSelect={onHide}>Hide field</ActionList.Item> : null}
        </ActionList>
      </ActionMenu.Overlay>
    </ActionMenu>
  );
}

export function NewFieldDialog({ project, onClose }: { project: ProjectFull; onClose: () => void }) {
  const m = useProjectMutations(project);
  const [name, setName] = useState('');
  const [type, setType] = useState<'text' | 'number' | 'date' | 'single_select'>('text');
  const [options, setOptions] = useState('');
  const submit = async () => {
    const opts = options.split('\n').map((o) => o.trim()).filter(Boolean);
    await m.createField.mutateAsync({ name, data_type: type, options: type === 'single_select' ? (opts.length ? opts : ['Option 1']).map((o, i) => ({ name: o, color: ['GRAY', 'BLUE', 'GREEN', 'YELLOW', 'ORANGE', 'RED', 'PINK', 'PURPLE'][i % 8], description: '' })) : undefined });
    onClose();
  };
  return (
    <Dialog title="New field" onClose={onClose} footerButtons={[{ content: 'Cancel', onClick: onClose }, { content: 'Save', buttonType: 'primary', onClick: submit, disabled: !name.trim() }]}>
      <div className="form-stack">
        <FormControl required>
          <FormControl.Label>Field name</FormControl.Label>
          <TextInput block value={name} onChange={(e) => setName(e.target.value)} autoFocus />
        </FormControl>
        <FormControl>
          <FormControl.Label>Field type</FormControl.Label>
          <select className="select" value={type} onChange={(e) => setType(e.target.value as typeof type)}>
            <option value="text">Text</option>
            <option value="number">Number</option>
            <option value="date">Date</option>
            <option value="single_select">Single select</option>
          </select>
        </FormControl>
        {type === 'single_select' ? (
          <FormControl>
            <FormControl.Label>Options (one per line)</FormControl.Label>
            <Textarea block rows={4} value={options} onChange={(e) => setOptions(e.target.value)} placeholder={'P0\nP1\nP2'} />
          </FormControl>
        ) : null}
      </div>
    </Dialog>
  );
}

export function ItemPanel({ project, item, onClose }: { project: ProjectFull; item: ProjectItem; onClose: () => void }) {
  const m = useProjectMutations(project);
  const c = item.content;
  const canWrite = project.permissions.write;
  const [title, setTitle] = useState(c.title ?? '');
  const [body, setBody] = useState(c.body ?? '');
  const [repo, setRepo] = useState(project.repositories[0]?.full_name ?? '');
  return (
    <div className="item-panel">
      <div className="panel-head">
        <ContentIcon c={c} />
        {c.type === 'draft' && canWrite ? (
          <TextInput value={title} onChange={(e) => setTitle(e.target.value)} onBlur={() => title !== c.title && m.updateDraft.mutate({ item: item.id, title })} block aria-label="Title" />
        ) : (
          <strong style={{ flex: 1, fontSize: 18 }}>{itemTitle(item)}</strong>
        )}
        <button className="linkish" onClick={onClose} aria-label="Close">
          <XIcon />
        </button>
      </div>
      {c.url ? (
        <p className="small" style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          {c.state ? <Label variant={c.merged ? 'done' : c.state === 'closed' ? 'danger' : 'success'}>{c.merged ? 'Merged' : c.state === 'closed' ? 'Closed' : 'Open'}</Label> : null}
          <a href={c.url} target="_blank" rel="noreferrer">
            {c.repository} #{c.number} <LinkExternalIcon size={12} />
          </a>
          {c.author ? <span className="muted">opened by {c.author.login}</span> : null}
        </p>
      ) : null}
      <div className="panel-fields">
        {project.fields.map((f) => (
          <div key={f.id} className="panel-field">
            <span className="muted small">{f.name}</span>
            <FieldValueEditor project={project} item={item} field={f} canWrite={canWrite} />
          </div>
        ))}
        {c.assignees?.length ? (
          <div className="panel-field">
            <span className="muted small">Assignees</span>
            <BuiltinValue item={item} k="assignees" />
          </div>
        ) : null}
        {c.labels?.length ? (
          <div className="panel-field">
            <span className="muted small">Labels</span>
            <BuiltinValue item={item} k="labels" />
          </div>
        ) : null}
        {c.milestone ? (
          <div className="panel-field">
            <span className="muted small">Milestone</span>
            <span className="small">{c.milestone}</span>
          </div>
        ) : null}
      </div>
      {c.type === 'draft' ? (
        canWrite ? (
          <textarea className="panel-body" value={body} onChange={(e) => setBody(e.target.value)} onBlur={() => body !== (c.body ?? '') && m.updateDraft.mutate({ item: item.id, body })} placeholder="Add a description (markdown)" />
        ) : (
          <Markdown html={c.body ?? ''} />
        )
      ) : c.body_html ? (
        <Markdown html={c.body_html} />
      ) : c.type !== 'hidden' && c.type !== 'deleted' ? (
        <p className="muted small">No description provided.</p>
      ) : null}
      {(c.type === 'issue' || c.type === 'pull') && c.repository && c.number ? <ItemActivity repository={c.repository} number={c.number} state={c.state ?? 'open'} isPull={c.type === 'pull'} /> : null}
      {canWrite ? (
        <div className="panel-actions">
          {c.type === 'draft' ? (
            <div style={{ display: 'flex', gap: 4, alignItems: 'center', flexWrap: 'wrap' }}>
              <TextInput size="small" value={repo} onChange={(e) => setRepo(e.target.value)} placeholder="owner/repo" list={`repos-${project.id}`} style={{ width: 200 }} aria-label="Repository" />
              <datalist id={`repos-${project.id}`}>
                {project.repositories.map((r) => (
                  <option key={r.id} value={r.full_name} />
                ))}
              </datalist>
              <Button size="small" onClick={() => m.convert.mutate({ item: item.id, repo })} disabled={!repo.includes('/')} loading={m.convert.isPending}>
                Convert to issue
              </Button>
              {m.convert.isError ? <span className="small" style={{ color: 'var(--fgColor-danger)' }}>{(m.convert.error as Error).message}</span> : null}
            </div>
          ) : null}
          <div style={{ display: 'flex', gap: 4 }}>
            <Button size="small" variant="invisible" leadingVisual={ArchiveIcon} onClick={() => { m.archive.mutate({ item: item.id, archived: true }); onClose(); }}>
              Archive
            </Button>
            <Button size="small" variant="danger" leadingVisual={TrashIcon} onClick={() => { if (confirm('Remove this item from the project?')) { m.remove.mutate(item.id); onClose(); } }}>
              Delete from project
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}


interface IssueComment {
  id: number;
  user: User;
  body_html?: string;
  body: string;
  created_at: string;
}

/// Comments and close/reopen for an issue or pull request shown inside the project panel.
function ItemActivity({ repository, number, state, isPull }: { repository: string; number: number; state: 'open' | 'closed'; isPull: boolean }) {
  const { user } = useSession();
  const qc = useQueryClient();
  const key = ['issues', repository.toLowerCase(), number, 'comments'];
  const comments = useQuery({ queryKey: key, queryFn: () => api<IssueComment[]>(`/repos/${repository}/issues/${number}/comments?per_page=100`, { accept: 'application/vnd.github.html+json' }) });
  const [draft, setDraft] = useState('');
  const invalidate = () => {
    qc.invalidateQueries({ queryKey: key });
    qc.invalidateQueries({ queryKey: ['project'] });
    qc.invalidateQueries({ queryKey: ['issues', repository.toLowerCase()] });
  };
  const post = useMutation({ mutationFn: () => api(`/repos/${repository}/issues/${number}/comments`, { method: 'POST', json: { body: draft } }), onSuccess: () => { setDraft(''); invalidate(); } });
  const setState = useMutation({ mutationFn: (st: 'open' | 'closed') => api(`/repos/${repository}/issues/${number}`, { method: 'PATCH', json: { state: st } }), onSuccess: invalidate });
  return (
    <div className="panel-activity">
      <h3>Activity</h3>
      {comments.data?.map((cm) => (
        <div key={cm.id} className="panel-comment">
          <div className="muted small" style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
            <Avatar src={cm.user.avatar_url} size={16} alt="" /> <strong>{cm.user.login}</strong> <RelativeTime datetime={cm.created_at} />
          </div>
          <Markdown html={cm.body_html ?? cm.body} />
        </div>
      ))}
      {comments.data?.length === 0 ? <p className="muted small">No comments yet.</p> : null}
      {user ? (
        <div className="form-stack" style={{ gap: 6 }}>
          <textarea className="panel-body" style={{ minHeight: 80 }} value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Leave a comment" />
          <div style={{ display: 'flex', gap: 6, justifyContent: 'flex-end' }}>
            {!isPull ? (
              <Button size="small" onClick={() => setState.mutate(state === 'open' ? 'closed' : 'open')} loading={setState.isPending}>
                {state === 'open' ? 'Close issue' : 'Reopen issue'}
              </Button>
            ) : null}
            <Button size="small" variant="primary" disabled={!draft.trim()} onClick={() => post.mutate()} loading={post.isPending}>
              Comment
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
