import { Button, FormControl, ProgressBar, TextInput, Textarea } from '@primer/react';
import { MilestoneIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { useState } from 'react';
import { ButtonLink } from '../components/links';
import { ErrorFlash, Loading } from '../components/Loading';
import { api } from '../lib/api';
import { milestonesQuery, type Milestone } from '../lib/issues';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/milestones')({
  validateSearch: (s: Record<string, unknown>): { state?: string } => (typeof s.state === 'string' ? { state: s.state } : {}),
  component: Milestones,
});

function Milestones() {
  const repo = useRepo();
  const { state = 'open' } = Route.useSearch();
  const qc = useQueryClient();
  const full = repo.full_name;
  const list = useQuery(milestonesQuery(full, state));
  const [creating, setCreating] = useState(false);
  const [editing, setEditing] = useState<Milestone | null>(null);
  const [error, setError] = useState<unknown>(null);
  const invalidate = () => qc.invalidateQueries({ queryKey: ['milestones', full.toLowerCase()] });
  const save = useMutation({
    mutationFn: (v: { title: string; description: string; due_on: string | null; number?: number }) =>
      v.number ? api(`/repos/${full}/milestones/${v.number}`, { method: 'PATCH', json: v }) : api(`/repos/${full}/milestones`, { method: 'POST', json: v }),
    onSuccess: () => {
      setCreating(false);
      setEditing(null);
      invalidate();
    },
    onError: setError,
  });
  const setState = useMutation({ mutationFn: (m: Milestone) => api(`/repos/${full}/milestones/${m.number}`, { method: 'PATCH', json: { state: m.state === 'open' ? 'closed' : 'open' } }), onSuccess: invalidate, onError: setError });
  const remove = useMutation({ mutationFn: (m: Milestone) => api(`/repos/${full}/milestones/${m.number}`, { method: 'DELETE' }), onSuccess: invalidate, onError: setError });
  const canEdit = !!repo.permissions?.push;
  const p = { owner: repo.owner.login, repo: repo.name };
  const form = (m?: Milestone) => (
    <form
      className="form-stack"
      style={{ padding: 16, background: 'var(--bgColor-muted)', border: '1px solid var(--borderColor-default)', borderRadius: 6, marginBottom: 16 }}
      onSubmit={(e) => {
        e.preventDefault();
        const f = new FormData(e.currentTarget);
        const due = String(f.get('due_on') ?? '');
        save.mutate({ number: m?.number, title: String(f.get('title')), description: String(f.get('description') ?? ''), due_on: due ? new Date(due).toISOString() : null });
      }}
    >
      <FormControl required>
        <FormControl.Label>Title</FormControl.Label>
        <TextInput name="title" defaultValue={m?.title ?? ''} block autoFocus />
      </FormControl>
      <FormControl>
        <FormControl.Label>Due date (optional)</FormControl.Label>
        <TextInput name="due_on" type="date" defaultValue={m?.due_on ? m.due_on.slice(0, 10) : ''} />
      </FormControl>
      <FormControl>
        <FormControl.Label>Description</FormControl.Label>
        <Textarea name="description" defaultValue={m?.description ?? ''} block rows={3} />
      </FormControl>
      <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
        <Button type="button" onClick={() => { setCreating(false); setEditing(null); }}>
          Cancel
        </Button>
        <Button type="submit" variant="primary" loading={save.isPending}>
          {m ? 'Save changes' : 'Create milestone'}
        </Button>
      </div>
    </form>
  );
  return (
    <div>
      <div style={{ display: 'flex', gap: 8, marginBottom: 16, alignItems: 'center' }}>
        <ButtonLink to="/$owner/$repo/labels" params={p}>
          Labels
        </ButtonLink>
        <ButtonLink to="/$owner/$repo/milestones" params={p} aria-current="page">
          Milestones
        </ButtonLink>
        <span style={{ flex: 1 }} />
        {canEdit ? (
          <Button variant="primary" onClick={() => setCreating(true)}>
            New milestone
          </Button>
        ) : null}
      </div>
      {error ? <ErrorFlash error={error} /> : null}
      {creating ? form() : null}
      {list.isPending ? <Loading /> : null}
      <div className="key-list">
        <div className="item" style={{ background: 'var(--bgColor-muted)', gap: 16 }}>
          <Link to="/$owner/$repo/milestones" params={p} search={{ state: 'open' }} style={{ fontWeight: state === 'open' ? 600 : 400, color: 'var(--fgColor-default)' }}>
            Open
          </Link>
          <Link to="/$owner/$repo/milestones" params={p} search={{ state: 'closed' }} style={{ fontWeight: state === 'closed' ? 600 : 400, color: 'var(--fgColor-default)' }}>
            Closed
          </Link>
        </div>
        {list.data?.map((m) =>
          editing?.id === m.id ? (
            <div key={m.id} className="item" style={{ display: 'block' }}>
              {form(m)}
            </div>
          ) : (
            <div key={m.id} className="item">
              <div className="body">
                <Link to="/$owner/$repo/issues" params={p} search={{ q: `is:issue is:open milestone:"${m.title}"`, page: 1 }} style={{ fontSize: 20, fontWeight: 600, color: 'var(--fgColor-default)' }}>
                  {m.title}
                </Link>
                <div className="muted small" style={{ margin: '4px 0' }}>
                  {m.due_on ? `Due by ${new Date(m.due_on).toLocaleDateString()}` : 'No due date'}
                  {m.description ? ` · ${m.description}` : ''}
                </div>
              </div>
              <div style={{ width: 240 }}>
                <ProgressBar progress={m.open_issues + m.closed_issues ? (m.closed_issues * 100) / (m.open_issues + m.closed_issues) : 0} aria-label="Completion" />
                <div className="muted small" style={{ marginTop: 4, display: 'flex', gap: 12 }}>
                  <span>
                    <strong>{m.open_issues + m.closed_issues ? Math.round((m.closed_issues * 100) / (m.open_issues + m.closed_issues)) : 0}%</strong> complete
                  </span>
                  <span>
                    <strong>{m.open_issues}</strong> open
                  </span>
                  <span>
                    <strong>{m.closed_issues}</strong> closed
                  </span>
                </div>
                {canEdit ? (
                  <div style={{ display: 'flex', gap: 4, marginTop: 4 }}>
                    <Button size="small" variant="invisible" onClick={() => setEditing(m)}>
                      Edit
                    </Button>
                    <Button size="small" variant="invisible" onClick={() => setState.mutate(m)}>
                      {m.state === 'open' ? 'Close' : 'Reopen'}
                    </Button>
                    <Button size="small" variant="invisible" onClick={() => confirm(`Delete milestone "${m.title}"?`) && remove.mutate(m)}>
                      Delete
                    </Button>
                  </div>
                ) : null}
              </div>
            </div>
          ),
        )}
        {list.data?.length === 0 ? (
          <div className="item" style={{ justifyContent: 'center', padding: 48, flexDirection: 'column', alignItems: 'center' }}>
            <MilestoneIcon size={24} className="muted" />
            <p className="muted">You haven't created any {state} milestones.</p>
          </div>
        ) : null}
      </div>
    </div>
  );
}
