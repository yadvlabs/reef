// Project status updates: the pill in the header, the "Add status update" dialog and the history list.
import { Avatar, Button, Dialog, FormControl, RelativeTime, Select, TextInput, Textarea } from '@primer/react';
import { TrashIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { reef } from '../../lib/api';
import { STATUS_UPDATE_STATES, statusUpdatesQuery, type ProjectFull, type StatusUpdate, type StatusUpdateState } from '../../lib/projects';
import { Markdown } from '../Markdown';

export function StatusPill({ update, size = 'small' }: { update: StatusUpdate | null | undefined; size?: 'small' | 'medium' }) {
  const meta = STATUS_UPDATE_STATES.find((s) => s.key === update?.status) ?? STATUS_UPDATE_STATES[0];
  return (
    <span className={`status-pill ${size}`} style={{ color: meta.color, background: meta.bg }}>
      <span className="dot" style={{ background: meta.color }} /> {update ? meta.label : 'No status'}
    </span>
  );
}

function useStatusMutations(project: ProjectFull) {
  const qc = useQueryClient();
  const base = `/projects/${project.owner.login}/${project.number}`;
  const invalidate = () => qc.invalidateQueries({ queryKey: ['project', project.owner.login.toLowerCase(), project.number] });
  const create = useMutation({ mutationFn: (v: { status: StatusUpdateState; start_date: string; target_date: string; body: string }) => reef<StatusUpdate>(`${base}/status_updates`, { method: 'POST', json: v }), onSuccess: invalidate });
  const remove = useMutation({ mutationFn: (id: number) => reef(`${base}/status_updates/${id}`, { method: 'DELETE' }), onSuccess: invalidate });
  return { create, remove };
}

export function StatusUpdateDialog({ project, onClose }: { project: ProjectFull; onClose: () => void }) {
  const m = useStatusMutations(project);
  const last = project.status_update ?? null;
  const [status, setStatus] = useState<StatusUpdateState>(last?.status ?? 'ON_TRACK');
  const [start, setStart] = useState(last?.start_date ?? '');
  const [target, setTarget] = useState(last?.target_date ?? '');
  const [body, setBody] = useState('');
  return (
    <Dialog title="Add status update" onClose={onClose} width="large">
      <form
        className="form-stack"
        onSubmit={(e) => {
          e.preventDefault();
          m.create.mutate({ status, start_date: start, target_date: target, body }, { onSuccess: onClose });
        }}
      >
        <div className="status-form-row">
          <FormControl>
            <FormControl.Label>Status</FormControl.Label>
            <Select value={status} onChange={(e) => setStatus(e.target.value as StatusUpdateState)}>
              {STATUS_UPDATE_STATES.map((s) => (
                <Select.Option key={s.key} value={s.key}>
                  {s.label}
                </Select.Option>
              ))}
            </Select>
          </FormControl>
          <FormControl>
            <FormControl.Label>Start date</FormControl.Label>
            <TextInput type="date" value={start} onChange={(e) => setStart(e.target.value)} />
          </FormControl>
          <FormControl>
            <FormControl.Label>Target date</FormControl.Label>
            <TextInput type="date" value={target} onChange={(e) => setTarget(e.target.value)} />
          </FormControl>
        </div>
        <FormControl>
          <FormControl.Label>Update</FormControl.Label>
          <Textarea value={body} onChange={(e) => setBody(e.target.value)} placeholder="What's the latest? Markdown is supported." rows={6} block />
        </FormControl>
        {m.create.isError ? <p style={{ color: 'var(--fgColor-danger)' }}>{(m.create.error as Error).message}</p> : null}
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" loading={m.create.isPending}>
            Save update
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function StatusUpdatesList({ project }: { project: ProjectFull }) {
  const q = useQuery(statusUpdatesQuery(project.owner.login, project.number));
  const m = useStatusMutations(project);
  const [adding, setAdding] = useState(false);
  const list = q.data ?? [];
  return (
    <div className="page narrow">
      <div className="status-list-head">
        <h2>Status updates</h2>
        <span style={{ flex: 1 }} />
        {project.permissions.write ? (
          <Button variant="primary" size="small" onClick={() => setAdding(true)}>
            Add status update
          </Button>
        ) : null}
      </div>
      {list.length === 0 && q.data ? <p className="muted">No status updates yet. Share progress with everyone following this project.</p> : null}
      <div className="status-updates">
        {list.map((u) => (
          <article key={u.id} className="status-update">
            <header>
              {u.author ? <Avatar src={u.author.avatar_url} size={24} alt="" /> : null}
              <strong>{u.author?.login ?? 'someone'}</strong>
              <span className="muted small">
                posted <RelativeTime datetime={u.created_at} />
              </span>
              <span style={{ flex: 1 }} />
              <StatusPill update={u} />
              {project.permissions.write ? (
                <button className="linkish muted" aria-label="Delete status update" title="Delete" onClick={() => confirm('Delete this status update?') && m.remove.mutate(u.id)}>
                  <TrashIcon />
                </button>
              ) : null}
            </header>
            {u.start_date || u.target_date ? (
              <div className="muted small status-dates">
                {u.start_date ? <span>Start: {u.start_date}</span> : null}
                {u.target_date ? <span>Target: {u.target_date}</span> : null}
              </div>
            ) : null}
            {u.body_html ? <Markdown html={u.body_html} /> : null}
          </article>
        ))}
      </div>
      {adding ? <StatusUpdateDialog project={project} onClose={() => setAdding(false)} /> : null}
    </div>
  );
}
