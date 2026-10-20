import { ActionList, ActionMenu, Button, Checkbox, FormControl, TextInput, Textarea } from '@primer/react';
import { TableIcon, ProjectIcon, PlusIcon, KebabHorizontalIcon, SearchIcon, XIcon, ProjectRoadmapIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { Board, ItemPanel, NewFieldDialog, Table, useProjectMutations, viewItems } from '../components/ProjectBoard';
import { ProjectHeader } from '../components/projects/ProjectShell';
import { Roadmap } from '../components/projects/Roadmap';
import { ApiError, reef } from '../lib/api';
import { Link } from '@tanstack/react-router';
import { BUILTIN_FIELDS, OPTION_COLORS, projectQuery, type ProjectFull, type ProjectView } from '../lib/projects';

export const Route = createFileRoute('/$owner/projects/$number')({
  validateSearch: (s: Record<string, unknown>): { view?: number; settings?: boolean; item?: number } => ({ ...(typeof s.view === 'number' ? { view: s.view } : {}), ...(s.settings === true ? { settings: true } : {}), ...(typeof s.item === 'number' ? { item: s.item } : {}) }),
  component: ProjectPage,
});

function ProjectPage() {
  const { owner, number } = Route.useParams();
  const { view: viewNumber, settings, item: openParam } = Route.useSearch();
  const navigate = useNavigate();
  const q = useQuery(projectQuery(owner, Number(number)));
  const [open, setOpen] = useState<number | null>(openParam ?? null);
  const [filter, setFilter] = useState('');
  const [newField, setNewField] = useState(false);
  useEffect(() => setFilter(''), [viewNumber]);
  if (q.isPending) return <><AppHeader context={{ owner, section: { label: 'Projects', to: 'projects' } }} /><Loading /></>;
  if (q.isError) return <><AppHeader context={{ owner, section: { label: 'Projects', to: 'projects' } }} />{q.error instanceof ApiError && q.error.status === 404 ? <NotFound what="project" /> : <ErrorFlash error={q.error} />}</>;
  const p = q.data;
  const view = p.views.find((v) => v.number === viewNumber) ?? p.views[0];
  const openItem = p.items.find((i) => i.id === open) ?? null;
  const go = (v?: number, s?: boolean) => navigate({ to: '/$owner/projects/$number', params: { owner, number }, search: { ...(v ? { view: v } : {}), ...(s ? { settings: true } : {}) } });
  const shown = view ? viewItems(p, view, filter).length : 0;
  return (
    <>
      <AppHeader context={{ owner, section: { label: 'Projects', to: 'projects' } }} />
      <ProjectHeader project={p} onSettings={() => go(undefined, true)} />
      <div className="project-views">
        {p.views.map((v) => (
          <ViewTab key={v.id} project={p} view={v} active={view?.id === v.id && !settings} onSelect={() => go(v.number)} />
        ))}
        {p.permissions.write ? <NewViewButton project={p} onCreated={(n) => go(n)} /> : null}
      </div>
      {settings ? (
        <Settings project={p} />
      ) : !view ? (
        <p className="muted" style={{ padding: 32 }}>This project has no views.</p>
      ) : (
        <>
          <div className="view-toolbar">
            <div className="filter-box">
              <SearchIcon className="muted" />
              <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder='Filter by keyword or by field, e.g. is:open status:"In Progress" -label:bug' aria-label="Filter items" />
              {filter ? (
                <button className="linkish" onClick={() => setFilter('')} aria-label="Clear filter">
                  <XIcon />
                </button>
              ) : null}
            </div>
            <span className="muted small">{shown} {shown === 1 ? 'item' : 'items'}</span>
            {filter && p.permissions.write && filter !== (view.settings.filter ?? '') ? (
              <SaveFilter project={p} view={view} filter={filter} />
            ) : null}
          </div>
          {view.layout === 'board' ? (
            <Board project={p} view={view} filter={filter} onOpen={(it) => setOpen(it.id)} />
          ) : view.layout === 'roadmap' ? (
            <Roadmap project={p} view={view} filter={filter} onOpen={(it) => setOpen(it.id)} />
          ) : (
            <Table project={p} view={view} filter={filter} onOpen={(it) => setOpen(it.id)} onAddField={() => setNewField(true)} />
          )}
        </>
      )}
      {openItem ? <ItemPanel key={openItem.id} project={p} item={openItem} onClose={() => setOpen(null)} /> : null}
      {newField ? <NewFieldDialog project={p} onClose={() => setNewField(false)} /> : null}
    </>
  );
}

function SaveFilter({ project, view, filter }: { project: ProjectFull; view: ProjectView; filter: string }) {
  const m = useProjectMutations(project);
  return (
    <Button size="small" onClick={() => m.updateView.mutate({ view: view.id, patch: { settings: { filter } } })} loading={m.updateView.isPending}>
      Save filter to view
    </Button>
  );
}

function ViewTab({ project, view, active, onSelect }: { project: ProjectFull; view: ProjectView; active: boolean; onSelect: () => void }) {
  const m = useProjectMutations(project);
  const [renaming, setRenaming] = useState(false);
  const [name, setName] = useState(view.name);
  const s = view.settings;
  const canWrite = project.permissions.write;
  const singles = project.fields.filter((f) => f.data_type === 'single_select');
  const dates = project.fields.filter((f) => f.data_type === 'date');
  const patch = (settings: Partial<ProjectView['settings']>) => m.updateView.mutate({ view: view.id, patch: { settings } });
  const visible = s.visible_fields ?? (view.layout === 'board' ? ['assignees', 'labels'] : [...project.fields.map((f) => String(f.id)), 'assignees', 'labels', 'repository']);
  const allFields = [...project.fields.map((f) => ({ key: String(f.id), name: f.name })), ...BUILTIN_FIELDS];
  if (renaming) {
    return (
      <form
        className="view-tab active"
        onSubmit={(e) => {
          e.preventDefault();
          m.updateView.mutate({ view: view.id, patch: { name } });
          setRenaming(false);
        }}
      >
        <TextInput size="small" value={name} onChange={(e) => setName(e.target.value)} autoFocus onBlur={() => setRenaming(false)} aria-label="View name" />
      </form>
    );
  }
  return (
    <div className={`view-tab${active ? ' active' : ''}`}>
      <button className="view-tab-button" onClick={onSelect}>
        {view.layout === 'board' ? <ProjectIcon /> : view.layout === 'roadmap' ? <ProjectRoadmapIcon /> : <TableIcon />} {view.name}
      </button>
      {active && canWrite ? (
        <ActionMenu>
          <ActionMenu.Anchor>
            <button className="linkish muted view-menu" aria-label="View options">
              <KebabHorizontalIcon />
            </button>
          </ActionMenu.Anchor>
          <ActionMenu.Overlay width="medium">
            <ActionList>
              <ActionList.Group>
                <ActionList.GroupHeading>Layout</ActionList.GroupHeading>
                <ActionList.Item selected={view.layout === 'table'} onSelect={() => m.updateView.mutate({ view: view.id, patch: { layout: 'table' } })}>
                  <ActionList.LeadingVisual>
                    <TableIcon />
                  </ActionList.LeadingVisual>
                  Table
                </ActionList.Item>
                <ActionList.Item selected={view.layout === 'board'} onSelect={() => m.updateView.mutate({ view: view.id, patch: { layout: 'board' } })}>
                  <ActionList.LeadingVisual>
                    <ProjectIcon />
                  </ActionList.LeadingVisual>
                  Board
                </ActionList.Item>
                <ActionList.Item selected={view.layout === 'roadmap'} onSelect={() => m.updateView.mutate({ view: view.id, patch: { layout: 'roadmap' } })}>
                  <ActionList.LeadingVisual>
                    <ProjectRoadmapIcon />
                  </ActionList.LeadingVisual>
                  Roadmap
                </ActionList.Item>
              </ActionList.Group>
              {view.layout === 'roadmap' ? (
                <>
                  <ActionList.Divider />
                  <ActionList.Group selectionVariant="single">
                    <ActionList.GroupHeading>Start date field</ActionList.GroupHeading>
                    {dates.map((f) => (
                      <ActionList.Item key={f.id} selected={s.start_field_id === f.id} onSelect={() => patch({ start_field_id: f.id })}>
                        {f.name}
                      </ActionList.Item>
                    ))}
                  </ActionList.Group>
                  <ActionList.Group selectionVariant="single">
                    <ActionList.GroupHeading>Target date field</ActionList.GroupHeading>
                    {dates.map((f) => (
                      <ActionList.Item key={f.id} selected={s.target_field_id === f.id} onSelect={() => patch({ target_field_id: f.id })}>
                        {f.name}
                      </ActionList.Item>
                    ))}
                  </ActionList.Group>
                  <ActionList.Group selectionVariant="single">
                    <ActionList.GroupHeading>Zoom</ActionList.GroupHeading>
                    {(['month', 'quarter', 'year'] as const).map((z) => (
                      <ActionList.Item key={z} selected={(s.zoom ?? 'month') === z} onSelect={() => patch({ zoom: z })}>
                        {z[0].toUpperCase() + z.slice(1)}
                      </ActionList.Item>
                    ))}
                  </ActionList.Group>
                </>
              ) : null}
              <ActionList.Divider />
              <ActionList.Group selectionVariant="single">
                <ActionList.GroupHeading>{view.layout === 'board' ? 'Column field' : 'Group by'}</ActionList.GroupHeading>
                {view.layout !== 'board' ? (
                  <ActionList.Item selected={!s.group_by_field_id} onSelect={() => patch({ group_by_field_id: null })}>
                    No grouping
                  </ActionList.Item>
                ) : null}
                {singles.map((f) => (
                  <ActionList.Item
                    key={f.id}
                    selected={view.layout === 'board' ? (s.column_field_id ?? singles[0]?.id) === f.id : s.group_by_field_id === f.id}
                    onSelect={() => (view.layout === 'board' ? patch({ column_field_id: f.id, ...(s.group_by_field_id === f.id ? { group_by_field_id: null } : {}) }) : patch({ group_by_field_id: f.id }))}
                  >
                    {f.name}
                  </ActionList.Item>
                ))}
              </ActionList.Group>
              {view.layout === 'board' ? (
                <>
                  <ActionList.Divider />
                  <ActionList.Group selectionVariant="single">
                    <ActionList.GroupHeading>Swimlanes (group by)</ActionList.GroupHeading>
                    <ActionList.Item selected={!s.group_by_field_id} onSelect={() => patch({ group_by_field_id: null })}>
                      None
                    </ActionList.Item>
                    {singles
                      .filter((f) => f.id !== (s.column_field_id ?? singles[0]?.id))
                      .map((f) => (
                        <ActionList.Item key={f.id} selected={s.group_by_field_id === f.id} onSelect={() => patch({ group_by_field_id: f.id })}>
                          {f.name}
                        </ActionList.Item>
                      ))}
                  </ActionList.Group>
                </>
              ) : null}
              <ActionList.Divider />
              <ActionList.Group selectionVariant="single">
                <ActionList.GroupHeading>Sort by</ActionList.GroupHeading>
                <ActionList.Item selected={!s.sort} onSelect={() => patch({ sort: null })}>
                  Manual order
                </ActionList.Item>
                {[{ key: 'title', name: 'Title' }, ...allFields].map((f) => (
                  <ActionList.Item key={f.key} selected={s.sort?.field === f.key} onSelect={() => patch({ sort: { field: f.key, direction: s.sort?.field === f.key && s.sort.direction === 'asc' ? 'desc' : 'asc' } })}>
                    {f.name}
                    {s.sort?.field === f.key ? <ActionList.TrailingVisual>{s.sort.direction === 'asc' ? '↑' : '↓'}</ActionList.TrailingVisual> : null}
                  </ActionList.Item>
                ))}
              </ActionList.Group>
              <ActionList.Divider />
              <ActionList.Group selectionVariant="multiple">
                <ActionList.GroupHeading>{view.layout === 'board' ? 'Fields on cards' : 'Fields'}</ActionList.GroupHeading>
                {allFields.map((f) => (
                  <ActionList.Item key={f.key} selected={visible.includes(f.key)} onSelect={() => patch({ visible_fields: visible.includes(f.key) ? visible.filter((k) => k !== f.key) : [...visible, f.key] })}>
                    {f.name}
                  </ActionList.Item>
                ))}
              </ActionList.Group>
              <ActionList.Divider />
              <ActionList.Item onSelect={() => setRenaming(true)}>Rename view</ActionList.Item>
              {s.filter ? <ActionList.Item onSelect={() => patch({ filter: null })}>Clear saved filter ({s.filter})</ActionList.Item> : null}
              <ActionList.Item variant="danger" disabled={project.views.length <= 1} onSelect={() => confirm(`Delete view "${view.name}"?`) && m.deleteView.mutate(view.id)}>
                Delete view
              </ActionList.Item>
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
      ) : null}
    </div>
  );
}

function NewViewButton({ project, onCreated }: { project: ProjectFull; onCreated: (number: number) => void }) {
  const m = useProjectMutations(project);
  const status = project.fields.find((f) => f.data_type === 'single_select');
  const add = (layout: 'board' | 'table' | 'roadmap') =>
    m.createView.mutate(
      {
        name: layout === 'board' ? 'Board' : layout === 'roadmap' ? 'Roadmap' : 'Table',
        layout,
        settings: layout === 'board' ? { column_field_id: status?.id ?? null, visible_fields: ['assignees', 'labels'] } : layout === 'roadmap' ? { zoom: 'month' } : { visible_fields: [...project.fields.map((f) => String(f.id)), 'assignees', 'labels', 'repository'] },
      },
      { onSuccess: (v) => onCreated(v.number) },
    );
  return (
    <ActionMenu>
      <ActionMenu.Anchor>
        <button className="view-tab muted" aria-label="New view">
          <PlusIcon /> New view
        </button>
      </ActionMenu.Anchor>
      <ActionMenu.Overlay>
        <ActionList>
          <ActionList.Item onSelect={() => add('table')}>
            <ActionList.LeadingVisual>
              <TableIcon />
            </ActionList.LeadingVisual>
            Table
          </ActionList.Item>
          <ActionList.Item onSelect={() => add('board')}>
            <ActionList.LeadingVisual>
              <ProjectIcon />
            </ActionList.LeadingVisual>
            Board
          </ActionList.Item>
          <ActionList.Item onSelect={() => add('roadmap')}>
            <ActionList.LeadingVisual>
              <ProjectRoadmapIcon />
            </ActionList.LeadingVisual>
            Roadmap
          </ActionList.Item>
        </ActionList>
      </ActionMenu.Overlay>
    </ActionMenu>
  );
}

const COLORS = ['GRAY', 'BLUE', 'GREEN', 'YELLOW', 'ORANGE', 'RED', 'PINK', 'PURPLE'];

function Settings({ project }: { project: ProjectFull }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const base = `/projects/${project.owner.login}/${project.number}`;
  const key = ['project', project.owner.login.toLowerCase(), project.number];
  const [error, setError] = useState<unknown>(null);
  const [newField, setNewField] = useState(false);
  const inv = () => qc.invalidateQueries({ queryKey: key });
  const save = useMutation({ mutationFn: (v: Record<string, unknown>) => reef(base, { method: 'PATCH', json: v }), onSuccess: inv, onError: setError });
  const del = useMutation({ mutationFn: () => reef(base, { method: 'DELETE' }), onSuccess: () => navigate({ to: '/$owner/projects', params: { owner: project.owner.login }, search: {} }), onError: setError });
  const delField = useMutation({ mutationFn: (id: number) => reef(`${base}/fields/${id}`, { method: 'DELETE' }), onSuccess: inv, onError: setError });
  const updField = useMutation({ mutationFn: (v: { id: number; name?: string; options?: { id?: string; name: string; color: string; description: string }[] }) => reef(`${base}/fields/${v.id}`, { method: 'PATCH', json: { name: v.name, options: v.options } }), onSuccess: inv, onError: setError });
  const linkRepo = useMutation({ mutationFn: (repo: string) => reef(`${base}/repos`, { method: 'POST', json: { repo } }), onSuccess: inv, onError: setError });
  const unlinkRepo = useMutation({ mutationFn: (id: number) => reef(`${base}/repos/${id}`, { method: 'DELETE' }), onSuccess: inv, onError: setError });
  const [repo, setRepo] = useState('');
  return (
    <div className="page narrow form-stack">
      {error ? <ErrorFlash error={error} /> : null}
      <h2 className="section-title">Project settings</h2>
      <form
        className="form-stack"
        onSubmit={(e) => {
          e.preventDefault();
          const f = new FormData(e.currentTarget);
          save.mutate({ title: f.get('title'), short_description: f.get('description'), readme: f.get('readme'), public: f.get('public') === 'on' });
        }}
      >
        <FormControl required>
          <FormControl.Label>Project name</FormControl.Label>
          <TextInput name="title" defaultValue={project.title} block />
        </FormControl>
        <FormControl>
          <FormControl.Label>Short description</FormControl.Label>
          <TextInput name="description" defaultValue={project.short_description ?? ''} block />
        </FormControl>
        <FormControl>
          <FormControl.Label>README</FormControl.Label>
          <Textarea name="readme" defaultValue={project.readme ?? ''} block rows={5} />
        </FormControl>
        <FormControl>
          <Checkbox name="public" defaultChecked={project.public} />
          <FormControl.Label>Public</FormControl.Label>
          <FormControl.Caption>Anyone can see a public project; otherwise only the owner and members.</FormControl.Caption>
        </FormControl>
        <div>
          <Button type="submit" variant="primary" loading={save.isPending}>
            Save changes
          </Button>
        </div>
      </form>

      <h2 className="section-title" style={{ display: 'flex', alignItems: 'center' }}>
        Fields <span style={{ flex: 1 }} />
        <Button size="small" leadingVisual={PlusIcon} onClick={() => setNewField(true)}>
          New field
        </Button>
      </h2>
      <div className="key-list">
        {project.fields.map((f) => (
          <div key={f.id} className="item" style={{ alignItems: 'flex-start' }}>
            <div className="body">
              <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
                <input className="cell-input" style={{ fontWeight: 600, width: 220 }} defaultValue={f.name} aria-label="Field name" onBlur={(e) => e.target.value.trim() && e.target.value !== f.name && updField.mutate({ id: f.id, name: e.target.value.trim() })} />
                <span className="muted small">{f.data_type.replace('_', ' ')}</span>
              </div>
              {f.data_type === 'single_select' ? (
                <div style={{ marginTop: 8, display: 'flex', flexDirection: 'column', gap: 6 }}>
                  {f.options.map((o) => (
                    <div key={o.id} className="option-row">
                      <span className="dot" style={{ background: OPTION_COLORS[o.color] ?? OPTION_COLORS.GRAY, border: '1px solid var(--borderColor-default)' }} />
                      <input className="cell-input" style={{ width: 160 }} defaultValue={o.name} aria-label="Option name" onBlur={(e) => e.target.value.trim() !== o.name && updField.mutate({ id: f.id, options: f.options.map((x) => (x.id === o.id ? { ...x, name: e.target.value.trim() || x.name } : x)) })} />
                      <select className="select small" value={o.color} onChange={(e) => updField.mutate({ id: f.id, options: f.options.map((x) => (x.id === o.id ? { ...x, color: e.target.value } : x)) })} aria-label="Colour">
                        {COLORS.map((c) => (
                          <option key={c} value={c}>
                            {c.toLowerCase()}
                          </option>
                        ))}
                      </select>
                      <input className="cell-input" style={{ flex: 1 }} defaultValue={o.description} placeholder="Description" aria-label="Description" onBlur={(e) => e.target.value !== o.description && updField.mutate({ id: f.id, options: f.options.map((x) => (x.id === o.id ? { ...x, description: e.target.value } : x)) })} />
                      <button className="linkish muted" aria-label="Remove option" onClick={() => confirm(`Remove option "${o.name}"?`) && updField.mutate({ id: f.id, options: f.options.filter((x) => x.id !== o.id) })}>
                        <XIcon />
                      </button>
                    </div>
                  ))}
                  <form
                    style={{ display: 'flex', gap: 4 }}
                    onSubmit={(e) => {
                      e.preventDefault();
                      const fd = new FormData(e.currentTarget);
                      const name = String(fd.get('opt') ?? '').trim();
                      if (!name) return;
                      updField.mutate({ id: f.id, options: [...f.options, { name, color: COLORS[f.options.length % COLORS.length], description: '' }] });
                      (e.target as HTMLFormElement).reset();
                    }}
                  >
                    <TextInput name="opt" size="small" placeholder="Add option" />
                    <Button type="submit" size="small">
                      Add
                    </Button>
                  </form>
                </div>
              ) : null}
            </div>
            <Button size="small" variant="danger" onClick={() => confirm(`Delete field "${f.name}" and its values?`) && delField.mutate(f.id)}>
              Delete
            </Button>
          </div>
        ))}
      </div>
      {newField ? <NewFieldDialog project={project} onClose={() => setNewField(false)} /> : null}

      <h2 className="section-title">Linked repositories</h2>
      <p className="muted small">Issues and pull requests from linked repositories can be added by number, and the repository appears in the project's Add item search.</p>
      <div className="key-list">
        {project.repositories.map((r) => (
          <div key={r.id} className="item" style={{ alignItems: 'center' }}>
            <div className="body">{r.full_name}</div>
            <Button size="small" onClick={() => unlinkRepo.mutate(r.id)}>
              Unlink
            </Button>
          </div>
        ))}
      </div>
      <form
        style={{ display: 'flex', gap: 8 }}
        onSubmit={(e) => {
          e.preventDefault();
          if (repo.trim()) linkRepo.mutate(repo.trim());
          setRepo('');
        }}
      >
        <TextInput value={repo} onChange={(e) => setRepo(e.target.value)} placeholder="owner/repo" />
        <Button type="submit">Link repository</Button>
      </form>

      <h2 className="section-title">Workflows and archive</h2>
      <p className="muted small">
        Automations (status on close, reopen, merge, review; auto-add; auto-archive) live on the{' '}
        <Link to="/$owner/projects/$number/workflows" params={{ owner: project.owner.login, number: String(project.number) }}>
          Workflows
        </Link>{' '}
        page. Archived items can be restored from{' '}
        <Link to="/$owner/projects/$number/archive" params={{ owner: project.owner.login, number: String(project.number) }}>
          Archived items
        </Link>
        .
      </p>

      <h2 className="section-title" style={{ color: 'var(--fgColor-danger)' }}>Danger zone</h2>
      <div className="danger-zone">
        <div className="row">
          <div className="text">
            <strong>{project.closed ? 'Reopen' : 'Close'} project</strong>Closed projects are hidden from the open list.
          </div>
          <Button variant="danger" onClick={() => save.mutate({ closed: !project.closed })}>
            {project.closed ? 'Reopen' : 'Close'} project
          </Button>
        </div>
        <div className="row">
          <div className="text">
            <strong>Delete project</strong>This cannot be undone.
          </div>
          <Button variant="danger" onClick={() => confirm('Delete this project?') && del.mutate()}>
            Delete project
          </Button>
        </div>
      </div>
    </div>
  );
}
