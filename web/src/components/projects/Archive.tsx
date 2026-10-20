// Archived items: search, restore or delete one at a time or in bulk.
import { Button, Checkbox, RelativeTime, TextInput } from '@primer/react';
import { ArchiveIcon, SearchIcon, TrashIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { reef } from '../../lib/api';
import { archiveQuery, itemTitle, type ProjectFull } from '../../lib/projects';
import { ErrorFlash, Loading } from '../Loading';
import { ContentIcon } from '../ProjectBoard';

export function Archive({ project }: { project: ProjectFull }) {
  const qc = useQueryClient();
  const [q, setQ] = useState('');
  const list = useQuery(archiveQuery(project.owner.login, project.number, q));
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const base = `/projects/${project.owner.login}/${project.number}`;
  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ['project', project.owner.login.toLowerCase(), project.number] });
    setSelected(new Set());
  };
  const restore = useMutation({ mutationFn: (ids: number[]) => Promise.all(ids.map((id) => reef(`${base}/items/${id}`, { method: 'PATCH', json: { archived: false } }))), onSuccess: invalidate });
  const remove = useMutation({ mutationFn: (ids: number[]) => Promise.all(ids.map((id) => reef(`${base}/items/${id}`, { method: 'DELETE' }))), onSuccess: invalidate });
  const canWrite = project.permissions.write;
  const items = list.data ?? [];
  const allSelected = items.length > 0 && items.every((i) => selected.has(i.id));
  return (
    <div className="page narrow">
      <div className="archive-head">
        <h2>Archived items</h2>
        <span className="muted small">{items.length} {items.length === 1 ? 'item' : 'items'}</span>
      </div>
      <TextInput leadingVisual={SearchIcon} value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search archived items" block aria-label="Search archived items" />
      {canWrite && items.length ? (
        <div className="archive-bulk">
          <Checkbox checked={allSelected} onChange={() => setSelected(allSelected ? new Set() : new Set(items.map((i) => i.id)))} aria-label="Select all" />
          <span className="muted small">{selected.size ? `${selected.size} selected` : 'Select all'}</span>
          <span style={{ flex: 1 }} />
          <Button size="small" leadingVisual={ArchiveIcon} disabled={!selected.size} onClick={() => restore.mutate([...selected])} loading={restore.isPending}>
            Restore
          </Button>
          <Button size="small" variant="danger" leadingVisual={TrashIcon} disabled={!selected.size} onClick={() => confirm(`Delete ${selected.size} item(s) from the project?`) && remove.mutate([...selected])} loading={remove.isPending}>
            Delete
          </Button>
        </div>
      ) : null}
      {list.isPending ? <Loading /> : list.isError ? <ErrorFlash error={list.error} /> : null}
      {list.data && items.length === 0 ? (
        <Blankslate border>
          <Blankslate.Visual>
            <ArchiveIcon size="medium" />
          </Blankslate.Visual>
          <Blankslate.Heading>No archived items</Blankslate.Heading>
          <Blankslate.Description>{q ? 'Nothing matches your search.' : 'Archive items from a card or the item panel, or let the "Auto-archive items" workflow do it.'}</Blankslate.Description>
        </Blankslate>
      ) : null}
      <div className="archive-list">
        {items.map((it) => (
          <div key={it.id} className="archive-row">
            {canWrite ? <Checkbox checked={selected.has(it.id)} onChange={() => setSelected((s) => { const n = new Set(s); if (n.has(it.id)) n.delete(it.id); else n.add(it.id); return n; })} aria-label="Select item" /> : null}
            <ContentIcon c={it.content} />
            <div className="body">
              <div>
                {it.content.url ? (
                  <a href={it.content.url}>{itemTitle(it)}</a>
                ) : (
                  <strong>{itemTitle(it)}</strong>
                )}
              </div>
              <div className="muted small">
                {it.content.repository ? `${it.content.repository} #${it.content.number} · ` : 'Draft · '}archived <RelativeTime datetime={it.updated_at} />
              </div>
            </div>
            {canWrite ? (
              <div style={{ display: 'flex', gap: 4 }}>
                <Button size="small" onClick={() => restore.mutate([it.id])}>
                  Restore
                </Button>
                <Button size="small" variant="danger" onClick={() => confirm('Delete this item from the project?') && remove.mutate([it.id])}>
                  Delete
                </Button>
              </div>
            ) : null}
          </div>
        ))}
      </div>
    </div>
  );
}
