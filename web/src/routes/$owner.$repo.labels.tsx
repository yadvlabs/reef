import { Button, FormControl, TextInput } from '@primer/react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { LabelBadge } from '../components/IssueBits';
import { ButtonLink } from '../components/links';
import { ErrorFlash, Loading } from '../components/Loading';
import { api } from '../lib/api';
import { labelsQuery, type Label } from '../lib/issues';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/labels')({ component: Labels });

function randomColor() {
  return Math.floor(Math.random() * 0xffffff)
    .toString(16)
    .padStart(6, '0');
}

function LabelForm({ initial, onSave, onCancel, busy }: { initial?: Label; onSave: (v: { name: string; color: string; description: string }) => void; onCancel: () => void; busy: boolean }) {
  const [name, setName] = useState(initial?.name ?? '');
  const [color, setColor] = useState(initial?.color ?? randomColor());
  const [description, setDescription] = useState(initial?.description ?? '');
  const preview: Label = { id: 0, node_id: '', name: name || 'Label preview', color: /^[0-9a-f]{6}$/i.test(color) ? color : 'ededed', description: null, default: false };
  return (
    <form
      style={{ padding: 16, background: 'var(--bgColor-muted)', border: '1px solid var(--borderColor-default)', borderRadius: 6, marginBottom: 16 }}
      onSubmit={(e) => {
        e.preventDefault();
        onSave({ name, color, description });
      }}
    >
      <div style={{ marginBottom: 12 }}>
        <LabelBadge label={preview} />
      </div>
      <div style={{ display: 'flex', gap: 12, alignItems: 'flex-end', flexWrap: 'wrap' }}>
        <FormControl required>
          <FormControl.Label>Label name</FormControl.Label>
          <TextInput value={name} onChange={(e) => setName(e.target.value)} placeholder="Label name" />
        </FormControl>
        <FormControl>
          <FormControl.Label>Description</FormControl.Label>
          <TextInput value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Description (optional)" style={{ width: 280 }} />
        </FormControl>
        <FormControl>
          <FormControl.Label>Color</FormControl.Label>
          <div style={{ display: 'flex', gap: 4 }}>
            <Button type="button" onClick={() => setColor(randomColor())} style={{ background: `#${color}` }} aria-label="Random colour">
              ↻
            </Button>
            <TextInput value={`#${color}`} onChange={(e) => setColor(e.target.value.replace('#', ''))} monospace style={{ width: 100 }} />
          </div>
        </FormControl>
        <div style={{ display: 'flex', gap: 8, marginLeft: 'auto' }}>
          <Button type="button" onClick={onCancel}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={!name.trim()} loading={busy}>
            {initial ? 'Save changes' : 'Create label'}
          </Button>
        </div>
      </div>
    </form>
  );
}

function Labels() {
  const repo = useRepo();
  const qc = useQueryClient();
  const full = repo.full_name;
  const labels = useQuery(labelsQuery(full));
  const [creating, setCreating] = useState(false);
  const [editing, setEditing] = useState<Label | null>(null);
  const [error, setError] = useState<unknown>(null);
  const invalidate = () => qc.invalidateQueries({ queryKey: ['labels', full.toLowerCase()] });
  const create = useMutation({ mutationFn: (v: { name: string; color: string; description: string }) => api(`/repos/${full}/labels`, { method: 'POST', json: v }), onSuccess: () => { setCreating(false); invalidate(); }, onError: setError });
  const update = useMutation({ mutationFn: (v: { name: string; color: string; description: string }) => api(`/repos/${full}/labels/${encodeURIComponent(editing!.name)}`, { method: 'PATCH', json: { new_name: v.name, color: v.color, description: v.description } }), onSuccess: () => { setEditing(null); invalidate(); }, onError: setError });
  const remove = useMutation({ mutationFn: (l: Label) => api(`/repos/${full}/labels/${encodeURIComponent(l.name)}`, { method: 'DELETE' }), onSuccess: invalidate, onError: setError });
  const canEdit = !!repo.permissions?.triage;
  const p = { owner: repo.owner.login, repo: repo.name };
  return (
    <div>
      <div style={{ display: 'flex', gap: 8, marginBottom: 16, alignItems: 'center' }}>
        <ButtonLink to="/$owner/$repo/labels" params={p} aria-current="page">
          Labels
        </ButtonLink>
        <ButtonLink to="/$owner/$repo/milestones" params={p}>
          Milestones
        </ButtonLink>
        <span style={{ flex: 1 }} />
        {canEdit ? (
          <Button variant="primary" onClick={() => setCreating(true)}>
            New label
          </Button>
        ) : null}
      </div>
      {error ? <ErrorFlash error={error} /> : null}
      {creating ? <LabelForm onSave={(v) => create.mutate(v)} onCancel={() => setCreating(false)} busy={create.isPending} /> : null}
      {labels.isPending ? <Loading /> : null}
      <div className="key-list">
        <div className="item" style={{ background: 'var(--bgColor-muted)', fontWeight: 600 }}>
          {labels.data?.length ?? 0} labels
        </div>
        {labels.data?.map((l) =>
          editing?.id === l.id ? (
            <div key={l.id} className="item" style={{ display: 'block' }}>
              <LabelForm initial={l} onSave={(v) => update.mutate(v)} onCancel={() => setEditing(null)} busy={update.isPending} />
            </div>
          ) : (
            <div key={l.id} className="item" style={{ alignItems: 'center' }}>
              <div style={{ width: '25%' }}>
                <LabelBadge label={l} />
              </div>
              <div className="body muted small">{l.description}</div>
              {canEdit ? (
                <div style={{ display: 'flex', gap: 4 }}>
                  <Button size="small" variant="invisible" onClick={() => setEditing(l)}>
                    Edit
                  </Button>
                  <Button size="small" variant="invisible" onClick={() => confirm(`Delete the label "${l.name}"? It will be removed from all issues.`) && remove.mutate(l)}>
                    Delete
                  </Button>
                </div>
              ) : null}
            </div>
          ),
        )}
      </div>
    </div>
  );
}
