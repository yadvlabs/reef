// Pinned repositories on a profile, and the picker to change them.
import { CrustyEmpty } from './Crusty';
import { Button, Checkbox, Dialog } from '@primer/react';
import { RepoIcon, PinIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { useState } from 'react';
import { api, reef, type Repo } from '../lib/api';
import { LANGUAGE_COLORS } from '../lib/repo';

export const pinsQuery = (owner: string) => ({ queryKey: ['account', owner.toLowerCase(), 'pins'], queryFn: () => reef<Repo[]>(`/users/${owner}/pins`) });

export function PinnedRepos({ owner, fallback, canEdit, isOrg }: { owner: string; fallback: Repo[]; canEdit: boolean; isOrg: boolean }) {
  const pins = useQuery(pinsQuery(owner));
  const [editing, setEditing] = useState(false);
  const list = pins.data?.length ? pins.data : fallback.slice(0, 6);
  const pinned = !!pins.data?.length;
  return (
    <section>
      <h2 className="overview-title" style={{ display: 'flex', alignItems: 'center' }}>
        {pinned ? 'Pinned' : isOrg ? 'Repositories' : 'Popular repositories'}
        <span style={{ flex: 1 }} />
        {canEdit ? (
          <button className="linkish small" onClick={() => setEditing(true)}>
            Customize your pins
          </button>
        ) : null}
      </h2>
      <div className="pinned-grid">
        {list.map((r) => (
          <div key={r.id} className="pinned">
            <div className="headline">
              <RepoIcon className="muted" />
              <Link to="/$owner/$repo" params={{ owner: r.owner.login, repo: r.name }}>
                {r.name}
              </Link>
              <span className="visibility-pill">{r.private ? 'Private' : 'Public'}</span>
            </div>
            <p className="desc muted small">{r.description ?? ''}</p>
            <div className="muted small" style={{ display: 'flex', gap: 12 }}>
              <RepoLang repo={r} />
              {r.stargazers_count ? <span>★ {r.stargazers_count}</span> : null}
              {r.forks_count ? <span>⑂ {r.forks_count}</span> : null}
            </div>
          </div>
        ))}
        {list.length === 0 ? <CrustyEmpty mood="builder" size={80} title="No repositories yet." /> : null}
      </div>
      {editing ? <PinsDialog owner={owner} onClose={() => setEditing(false)} /> : null}
    </section>
  );
}

function RepoLang({ repo }: { repo: Repo }) {
  const q = useQuery({ queryKey: ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase(), 'languages'], queryFn: () => api<Record<string, number>>(`/repos/${repo.full_name}/languages`), staleTime: 300_000 });
  const top = Object.entries(q.data ?? {}).sort((x, y) => y[1] - x[1])[0];
  if (!top) return null;
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
      <span style={{ width: 10, height: 10, borderRadius: '50%', background: LANGUAGE_COLORS[top[0]] ?? '#ededed', display: 'inline-block' }} /> {top[0]}
    </span>
  );
}

export function PinsEditor({ owner, isOrg, canEdit }: { owner: string; isOrg: boolean; canEdit: boolean }) {
  const qc = useQueryClient();
  const pins = useQuery(pinsQuery(owner));
  const repos = useQuery({ queryKey: ['account', owner.toLowerCase(), 'repos'], queryFn: () => api<Repo[]>(`/${isOrg ? 'orgs' : 'users'}/${owner}/repos?sort=updated&per_page=100`) });
  const [selected, setSelected] = useState<string[] | null>(null);
  const current = selected ?? (pins.data ?? []).map((r) => r.name);
  const save = useMutation({ mutationFn: (names: string[]) => reef(`/users/${owner}/pins`, { method: 'PUT', json: { repos: names } }), onSuccess: () => qc.invalidateQueries({ queryKey: ['account', owner.toLowerCase(), 'pins'] }) });
  const toggle = (n: string) => setSelected(current.includes(n) ? current.filter((x) => x !== n) : current.length >= 6 ? current : [...current, n]);
  return (
    <div className="form-stack">
      <div className="key-list">
        {repos.data?.map((r) => (
          <label key={r.id} className="item" style={{ alignItems: 'center', cursor: canEdit ? 'pointer' : 'default' }}>
            <Checkbox checked={current.includes(r.name)} onChange={() => toggle(r.name)} disabled={!canEdit} />
            <div className="body">
              <strong>{r.name}</strong> <span className="visibility-pill">{r.private ? 'Private' : 'Public'}</span>
              <div className="muted small">{r.description ?? ''}</div>
            </div>
          </label>
        ))}
      </div>
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        <Button variant="primary" onClick={() => save.mutate(current)} disabled={!canEdit || selected === null} loading={save.isPending}>
          Save pins
        </Button>
        <span className="muted small">{current.length}/6 selected</span>
        {save.isError ? <span className="small" style={{ color: 'var(--fgColor-danger)' }}>{(save.error as Error).message}</span> : null}
      </div>
    </div>
  );
}

function PinsDialog({ owner, onClose }: { owner: string; onClose: () => void }) {
  const isOrg = false;
  return (
    <Dialog title="Edit pinned items" subtitle="Select up to six public repositories you'd like to show." onClose={onClose} width="medium">
      <PinsEditor owner={owner} isOrg={isOrg} canEdit />
    </Dialog>
  );
}

export { PinIcon as _PinIcon };
