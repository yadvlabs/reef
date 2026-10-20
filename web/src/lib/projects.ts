import { queryOptions } from '@tanstack/react-query';
import { reef, type User } from './api';

export interface FieldOption {
  id: string;
  name: string;
  color: string;
  description: string;
}
export interface ProjectField {
  id: number;
  node_id: string;
  name: string;
  data_type: 'single_select' | 'text' | 'number' | 'date' | 'iteration';
  options: FieldOption[];
}
/// Per-view configuration, stored as JSON on the server.
export interface ViewSettings {
  /// Board columns (a single-select field id).
  column_field_id?: number | null;
  /// Swimlanes on a board, or row groups in a table (a single-select field id).
  group_by_field_id?: number | null;
  sort?: { field: string; direction: 'asc' | 'desc' } | null;
  /// Field keys shown as table columns / card chips: built-ins by name, custom by id.
  visible_fields?: string[];
  filter?: string | null;
  /// Roadmap: date fields for bar start and end, and the time scale.
  start_field_id?: number | null;
  target_field_id?: number | null;
  zoom?: 'month' | 'quarter' | 'year';
}
export interface ProjectView {
  id: number;
  number: number;
  name: string;
  layout: 'board' | 'table' | 'roadmap';
  group_by_field_id: number | null;
  filter: string | null;
  settings: ViewSettings;
}
export interface ItemContent {
  type: 'issue' | 'pull' | 'draft' | 'deleted' | 'hidden';
  id?: number;
  number?: number;
  title?: string;
  body?: string | null;
  body_html?: string;
  state?: 'open' | 'closed';
  state_reason?: string | null;
  merged?: boolean;
  repository?: string;
  url?: string;
  milestone?: string | null;
  labels?: { name: string; color: string }[];
  assignees?: User[];
  author?: User;
}
export interface ProjectItem {
  id: number;
  position: number;
  archived: boolean;
  content: ItemContent;
  values: Record<string, string | null>;
  created_at: string;
  updated_at: string;
}
export interface ProjectSummary {
  id: number;
  number: number;
  title: string;
  short_description: string | null;
  readme: string | null;
  public: boolean;
  closed: boolean;
  owner: User;
  url: string;
  created_at: string;
  updated_at: string;
  items_count?: number;
  status_update?: StatusUpdate | null;
  permissions: { read: boolean; write: boolean; admin: boolean };
}
export interface ProjectFull extends ProjectSummary {
  fields: ProjectField[];
  views: ProjectView[];
  items: ProjectItem[];
  repositories: { id: number; full_name: string }[];
  readme_html: string;
  archived_count?: number;
}

export type StatusUpdateState = 'INACTIVE' | 'ON_TRACK' | 'AT_RISK' | 'OFF_TRACK' | 'COMPLETE';
export interface StatusUpdate {
  id: number;
  status: StatusUpdateState;
  start_date: string | null;
  target_date: string | null;
  body: string | null;
  body_html: string;
  author: User | null;
  created_at: string;
}
export const STATUS_UPDATE_STATES: { key: StatusUpdateState; label: string; color: string; bg: string }[] = [
  { key: 'INACTIVE', label: 'Inactive', color: 'var(--fgColor-muted)', bg: 'var(--bgColor-neutral-muted)' },
  { key: 'ON_TRACK', label: 'On track', color: 'var(--fgColor-success)', bg: 'var(--bgColor-success-muted)' },
  { key: 'AT_RISK', label: 'At risk', color: 'var(--fgColor-attention)', bg: 'var(--bgColor-attention-muted)' },
  { key: 'OFF_TRACK', label: 'Off track', color: 'var(--fgColor-danger)', bg: 'var(--bgColor-danger-muted)' },
  { key: 'COMPLETE', label: 'Complete', color: 'var(--fgColor-done)', bg: 'var(--bgColor-done-muted)' },
];

export type WorkflowKind = 'item_added' | 'item_reopened' | 'item_closed' | 'changes_requested' | 'review_approved' | 'pull_merged' | 'auto_archive' | 'auto_add' | 'auto_close';
export interface ProjectWorkflow {
  kind: WorkflowKind;
  enabled: boolean;
  configured: boolean;
  config: { status?: string | null; filter?: string | null; repo?: string | null };
}
export const WORKFLOW_META: Record<WorkflowKind, { name: string; when: string; then: string; kind: 'status' | 'archive' | 'add' | 'close' }> = {
  item_added: { name: 'Item added to project', when: 'An issue, pull request or draft is added to the project', then: 'Set status', kind: 'status' },
  item_reopened: { name: 'Item reopened', when: 'An issue or pull request in the project is reopened', then: 'Set status', kind: 'status' },
  item_closed: { name: 'Item closed', when: 'An issue or pull request in the project is closed', then: 'Set status', kind: 'status' },
  changes_requested: { name: 'Code changes requested', when: 'A review on a pull request in the project requests changes', then: 'Set status', kind: 'status' },
  review_approved: { name: 'Code review approved', when: 'A review on a pull request in the project approves it', then: 'Set status', kind: 'status' },
  pull_merged: { name: 'Pull request merged', when: 'A pull request in the project is merged', then: 'Set status', kind: 'status' },
  auto_archive: { name: 'Auto-archive items', when: 'Items match this filter (checked every 30 seconds)', then: 'Archive the item', kind: 'archive' },
  auto_add: { name: 'Auto-add to project', when: 'Issues or pull requests in a repository match this filter', then: 'Add the item to the project', kind: 'add' },
  auto_close: { name: 'Auto-close issue', when: 'The status of an issue in the project changes to', then: 'Close the issue', kind: 'close' },
};

export interface ProjectInsights {
  since: string;
  until: string;
  days: number;
  status_options: { id: string | null; name: string; color: string }[];
  burnup: { date: string; counts: number[] }[];
  status: { field: ProjectField | null; buckets: { name: string; color: string; count: number }[]; total: number };
  field: { name: string; buckets: { name: string; color: string; count: number }[] } | null;
  fields: { key: string; name: string }[];
}
export const insightsQuery = (owner: string, number: number, range: string, field: string) =>
  queryOptions({ queryKey: ['project', owner.toLowerCase(), number, 'insights', range, field], queryFn: () => reef<ProjectInsights>(`/projects/${owner}/${number}/insights?range=${range}&field=${encodeURIComponent(field)}`), staleTime: 30_000 });
export const workflowsQuery = (owner: string, number: number) =>
  queryOptions({ queryKey: ['project', owner.toLowerCase(), number, 'workflows'], queryFn: () => reef<{ workflows: ProjectWorkflow[]; status_field: ProjectField | null }>(`/projects/${owner}/${number}/workflows`) });
export const archiveQuery = (owner: string, number: number, q: string) =>
  queryOptions({ queryKey: ['project', owner.toLowerCase(), number, 'archive', q], queryFn: () => reef<ProjectItem[]>(`/projects/${owner}/${number}/archive?q=${encodeURIComponent(q)}`) });
export const statusUpdatesQuery = (owner: string, number: number) =>
  queryOptions({ queryKey: ['project', owner.toLowerCase(), number, 'status_updates'], queryFn: () => reef<StatusUpdate[]>(`/projects/${owner}/${number}/status_updates`) });

export const projectsQuery = (owner: string, state = 'open') => queryOptions({ queryKey: ['projects', owner.toLowerCase(), state], queryFn: () => reef<ProjectSummary[]>(`/projects/${owner}?state=${state}`) });
export const projectQuery = (owner: string, number: number) => queryOptions({ queryKey: ['project', owner.toLowerCase(), number], queryFn: () => reef<ProjectFull>(`/projects/${owner}/${number}`) });

export const OPTION_COLORS: Record<string, string> = {
  GRAY: 'var(--bgColor-neutral-muted)',
  BLUE: 'var(--bgColor-accent-muted)',
  GREEN: 'var(--bgColor-success-muted)',
  YELLOW: 'var(--bgColor-attention-muted)',
  ORANGE: 'var(--bgColor-severe-muted)',
  RED: 'var(--bgColor-danger-muted)',
  PINK: 'var(--bgColor-sponsors-muted)',
  PURPLE: 'var(--bgColor-done-muted)',
};
export const OPTION_DOTS: Record<string, string> = {
  GRAY: 'var(--fgColor-muted)',
  BLUE: 'var(--fgColor-accent)',
  GREEN: 'var(--fgColor-success)',
  YELLOW: 'var(--fgColor-attention)',
  ORANGE: 'var(--fgColor-severe)',
  RED: 'var(--fgColor-danger)',
  PINK: 'var(--fgColor-sponsors)',
  PURPLE: 'var(--fgColor-done)',
};

/// Built-in columns every project has, alongside the custom fields.
export const BUILTIN_FIELDS: { key: string; name: string }[] = [
  { key: 'assignees', name: 'Assignees' },
  { key: 'labels', name: 'Labels' },
  { key: 'repository', name: 'Repository' },
  { key: 'milestone', name: 'Milestone' },
];

export function itemTitle(it: ProjectItem): string {
  const c = it.content;
  if (c.type === 'draft') return c.title ?? 'Untitled';
  if (c.type === 'hidden') return 'Restricted item';
  if (c.type === 'deleted') return 'Deleted item';
  return c.title ?? '';
}

export function optionName(f: ProjectField, id: string | null | undefined): string {
  return f.options.find((o) => o.id === id)?.name ?? '';
}

/// GitHub-style project filter: free text, `is:open|closed|issue|pr|draft|merged`,
/// `status:Todo`, `field:"two words"`, `no:status`, `label:x`, `assignee:me`, `repo:o/r`,
/// `-` negation. Unknown qualifiers match the field with that name.
export function parseFilter(q: string): (it: ProjectItem, fields: ProjectField[]) => boolean {
  const terms: { neg: boolean; key: string | null; value: string }[] = [];
  const re = /(-)?(?:([\w.-]+):)?("([^"]*)"|(\S+))/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(q))) {
    terms.push({ neg: !!m[1], key: m[2]?.toLowerCase() ?? null, value: (m[4] ?? m[5] ?? '').toLowerCase() });
  }
  return (it, fields) => {
    const c = it.content;
    for (const t of terms) {
      let ok: boolean;
      if (t.key === null) {
        ok = itemTitle(it).toLowerCase().includes(t.value) || (c.repository ?? '').toLowerCase().includes(t.value) || String(c.number ?? '').includes(t.value);
      } else if (t.key === 'is') {
        ok =
          t.value === 'open' ? c.state === 'open' || c.type === 'draft' :
          t.value === 'closed' ? c.state === 'closed' :
          t.value === 'issue' ? c.type === 'issue' :
          t.value === 'pr' || t.value === 'pull' ? c.type === 'pull' :
          t.value === 'draft' ? c.type === 'draft' :
          t.value === 'merged' ? !!c.merged : true;
      } else if (t.key === 'no') {
        const f = fields.find((x) => x.name.toLowerCase() === t.value);
        ok = f ? !it.values[String(f.id)] : t.value === 'assignee' ? !(c.assignees?.length) : t.value === 'label' ? !(c.labels?.length) : true;
      } else if (t.key === 'label') {
        ok = (c.labels ?? []).some((l) => l.name.toLowerCase() === t.value);
      } else if (t.key === 'assignee') {
        ok = (c.assignees ?? []).some((a) => a.login.toLowerCase() === t.value);
      } else if (t.key === 'repo' || t.key === 'repository') {
        ok = (c.repository ?? '').toLowerCase() === t.value || (c.repository ?? '').toLowerCase().endsWith('/' + t.value);
      } else if (t.key === 'milestone') {
        ok = (c.milestone ?? '').toLowerCase() === t.value;
      } else {
        const f = fields.find((x) => x.name.toLowerCase() === t.key);
        if (!f) ok = true;
        else if (f.data_type === 'single_select') ok = optionName(f, it.values[String(f.id)]).toLowerCase() === t.value;
        else ok = (it.values[String(f.id)] ?? '').toLowerCase().includes(t.value);
      }
      if (t.neg ? ok : !ok) return false;
    }
    return true;
  };
}

/// Sort items by a field key (custom id or built-in name).
export function sortItems(items: ProjectItem[], fields: ProjectField[], sort: ViewSettings['sort']): ProjectItem[] {
  if (!sort) return items;
  const f = fields.find((x) => String(x.id) === sort.field);
  const keyOf = (it: ProjectItem): string | number => {
    if (sort.field === 'title') return itemTitle(it).toLowerCase();
    if (sort.field === 'repository') return it.content.repository ?? '';
    if (sort.field === 'assignees') return (it.content.assignees ?? []).map((a) => a.login).join(',');
    if (sort.field === 'labels') return (it.content.labels ?? []).map((l) => l.name).join(',');
    if (sort.field === 'milestone') return it.content.milestone ?? '';
    if (!f) return 0;
    const v = it.values[String(f.id)];
    if (f.data_type === 'single_select') return v ? f.options.findIndex((o) => o.id === v) : Number.MAX_SAFE_INTEGER;
    if (f.data_type === 'number') return v ? Number(v) : Number.MAX_SAFE_INTEGER;
    return (v ?? '￿').toLowerCase();
  };
  const dir = sort.direction === 'desc' ? -1 : 1;
  return [...items].sort((a, b) => {
    const x = keyOf(a);
    const y = keyOf(b);
    return (x < y ? -1 : x > y ? 1 : 0) * dir;
  });
}
