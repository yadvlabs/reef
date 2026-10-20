import { Button, FormControl, RelativeTime, TextInput } from '@primer/react';
import { SyncIcon, TrashIcon, UploadIcon, LockIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { ErrorFlash } from '../components/Loading';
import { SettingsNav } from '../components/SettingsNav';
import { reef, humanSize } from '../lib/api';
import { lfsQuery, mirrorQuery, pushMirrorsQuery, type MirrorConfig } from '../lib/mirror';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/settings/mirror')({ component: Page });

function Status({ error, pending, ok }: { error: string | null; pending?: boolean; ok: string | null }) {
  if (error) return <span className="mirror-status error"><span className="dot" /> Failed</span>;
  if (pending) return <span className="mirror-status pending"><span className="dot" /> Pending</span>;
  if (ok) return <span className="mirror-status ok"><span className="dot" /> Synced <RelativeTime datetime={ok} /></span>;
  return <span className="mirror-status"><span className="dot" /> Never synced</span>;
}

function Page() {
  const repo = useRepo();
  const full = repo.full_name;
  const qc = useQueryClient();
  const mirror = useQuery(mirrorQuery(full));
  const pushes = useQuery({ ...pushMirrorsQuery(full), refetchInterval: (q) => (q.state.data?.push_mirrors.some((m) => m.pending) ? 3000 : false) });
  const lfs = useQuery(lfsQuery(full));
  const [error, setError] = useState<unknown>(null);
  const invalidate = () => {
    qc.invalidateQueries({ queryKey: mirrorQuery(full).queryKey });
    qc.invalidateQueries({ queryKey: pushMirrorsQuery(full).queryKey });
    qc.invalidateQueries({ queryKey: lfsQuery(full).queryKey });
    qc.invalidateQueries({ queryKey: ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase()] });
  };

  // pull mirror form
  const m = mirror.data?.mirror ?? null;
  const [url, setUrl] = useState('');
  const [user, setUser] = useState('');
  const [pw, setPw] = useState('');
  const [interval, setInterval_] = useState('60');
  useEffect(() => {
    if (mirror.data) {
      setUrl(mirror.data.mirror?.pull_url ?? '');
      setUser(mirror.data.mirror?.username ?? '');
      setInterval_(String(mirror.data.mirror?.interval_minutes ?? 60));
      setPw('');
    }
  }, [mirror.data]);
  const save = useMutation({
    mutationFn: () => reef<{ mirror: MirrorConfig }>(`/repos/${full}/mirror`, { method: 'PUT', json: { pull_url: url.trim(), username: user.trim() || null, password: pw === '' ? undefined : pw, interval_minutes: Number(interval) || 60 } }),
    onSuccess: invalidate,
    onError: setError,
  });
  const sync = useMutation({ mutationFn: () => reef(`/repos/${full}/mirror/sync`, { method: 'POST' }), onSuccess: () => setTimeout(invalidate, 1500), onError: setError });
  const remove = useMutation({ mutationFn: () => reef(`/repos/${full}/mirror`, { method: 'DELETE' }), onSuccess: invalidate, onError: setError });
  // poll while a sync is due/running
  useEffect(() => {
    if (!m || (m.last_synced_at && !m.last_error)) return;
    const t = setTimeout(invalidate, 4000);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [m?.last_synced_at, m?.last_error, m?.next_sync_at]);

  // push mirrors
  const [pUrl, setPUrl] = useState('');
  const [pUser, setPUser] = useState('');
  const [pPw, setPPw] = useState('');
  const addPush = useMutation({
    mutationFn: () => reef(`/repos/${full}/push-mirrors`, { method: 'POST', json: { url: pUrl.trim(), username: pUser.trim() || null, password: pPw || null } }),
    onSuccess: () => {
      setPUrl('');
      setPUser('');
      setPPw('');
      invalidate();
    },
    onError: setError,
  });
  const delPush = useMutation({ mutationFn: (id: number) => reef(`/repos/${full}/push-mirrors/${id}`, { method: 'DELETE' }), onSuccess: invalidate, onError: setError });
  const pushNow = useMutation({ mutationFn: (id: number) => reef(`/repos/${full}/push-mirrors/${id}/push`, { method: 'POST' }), onSuccess: () => setTimeout(invalidate, 1000), onError: setError });
  const unlock = useMutation({ mutationFn: (id: number) => reef(`/repos/${full}/lfs/locks/${id}`, { method: 'DELETE' }), onSuccess: invalidate, onError: setError });

  const allowFile = !!mirror.data?.allow_file_urls;
  return (
    <div className="settings-layout">
      <SettingsNav repo={repo} current="mirror" />
      <div className="form-stack">
        {error ? <ErrorFlash error={error} /> : null}

        <h2 className="section-title">Pull mirror</h2>
        <p className="muted small">
          Keep this repository as a read-only copy of another one. Reef fetches every branch and tag on the interval below; pushes to this repository are refused while the mirror is configured.
          {allowFile ? ' Local paths and file:// URLs are allowed on this instance.' : ''}
        </p>
        <div className="card form-stack">
          {m ? (
            <div style={{ display: 'flex', gap: 12, alignItems: 'center', flexWrap: 'wrap' }}>
              <Status error={m.last_error} ok={m.last_synced_at} />
              {m.next_sync_at ? (
                <span className="muted small">
                  next sync <RelativeTime datetime={m.next_sync_at} />
                </span>
              ) : null}
              <span style={{ flex: 1 }} />
              <Button size="small" leadingVisual={SyncIcon} onClick={() => sync.mutate()} loading={sync.isPending}>
                Sync now
              </Button>
              <Button size="small" variant="danger" leadingVisual={TrashIcon} onClick={() => confirm('Stop mirroring? The repository keeps its content and becomes writable again.') && remove.mutate()}>
                Stop mirroring
              </Button>
            </div>
          ) : null}
          {m?.last_error ? <div className="mirror-error">{m.last_error}</div> : null}
          <FormControl required>
            <FormControl.Label>Source repository URL</FormControl.Label>
            <TextInput block value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://github.com/owner/repo.git" monospace />
          </FormControl>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 120px', gap: 8 }}>
            <FormControl>
              <FormControl.Label>Username</FormControl.Label>
              <TextInput block value={user} onChange={(e) => setUser(e.target.value)} autoComplete="off" />
            </FormControl>
            <FormControl>
              <FormControl.Label>Password or token</FormControl.Label>
              <TextInput block type="password" value={pw} onChange={(e) => setPw(e.target.value)} placeholder={m?.has_password ? '(unchanged)' : ''} autoComplete="new-password" />
            </FormControl>
            <FormControl>
              <FormControl.Label>Interval (min)</FormControl.Label>
              <TextInput block type="number" min={1} value={interval} onChange={(e) => setInterval_(e.target.value)} />
            </FormControl>
          </div>
          <div>
            <Button variant="primary" onClick={() => save.mutate()} loading={save.isPending} disabled={!url.trim()}>
              {m ? 'Update mirror' : 'Start mirroring'}
            </Button>
          </div>
        </div>

        <h2 className="section-title">Push mirrors</h2>
        <p className="muted small">Every push to this repository is replicated to these remotes with <code>git push --mirror</code>. Failed pushes retry every five minutes.</p>
        <div className="key-list">
          {pushes.data?.push_mirrors.map((pm) => (
            <div key={pm.id} className="item" style={{ alignItems: 'center' }}>
              <div className="body">
                <code>{pm.url}</code>
                <div style={{ marginTop: 4 }}>
                  <Status error={pm.last_error} pending={pm.pending} ok={pm.last_pushed_at} />
                  {pm.username ? <span className="muted small"> · as {pm.username}</span> : null}
                </div>
                {pm.last_error ? <div className="mirror-error">{pm.last_error}</div> : null}
              </div>
              <Button size="small" leadingVisual={UploadIcon} onClick={() => pushNow.mutate(pm.id)}>
                Push now
              </Button>
              <Button size="small" variant="danger" leadingVisual={TrashIcon} onClick={() => confirm('Remove this push mirror?') && delPush.mutate(pm.id)}>
                Remove
              </Button>
            </div>
          ))}
          {pushes.data?.push_mirrors.length === 0 ? <p className="muted" style={{ padding: 16 }}>No push mirrors.</p> : null}
        </div>
        <div className="card form-stack">
          <h3 style={{ margin: 0, fontSize: 16 }}>Add a push mirror</h3>
          <FormControl required>
            <FormControl.Label>Remote URL</FormControl.Label>
            <TextInput block value={pUrl} onChange={(e) => setPUrl(e.target.value)} placeholder="https://gitlab.example.com/owner/repo.git" monospace />
          </FormControl>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
            <FormControl>
              <FormControl.Label>Username</FormControl.Label>
              <TextInput block value={pUser} onChange={(e) => setPUser(e.target.value)} autoComplete="off" />
            </FormControl>
            <FormControl>
              <FormControl.Label>Password or token</FormControl.Label>
              <TextInput block type="password" value={pPw} onChange={(e) => setPPw(e.target.value)} autoComplete="new-password" />
            </FormControl>
          </div>
          <div>
            <Button onClick={() => addPush.mutate()} loading={addPush.isPending} disabled={!pUrl.trim()}>
              Add push mirror
            </Button>
          </div>
        </div>

        <h2 className="section-title">Git LFS</h2>
        {lfs.data ? (
          <>
            <p className="muted small">
              {lfs.data.enabled ? 'Git LFS is enabled' : 'Git LFS is disabled on this server'} · {lfs.data.object_count} {lfs.data.object_count === 1 ? 'object' : 'objects'} · {humanSize(lfs.data.total_bytes)} · objects up to {humanSize(lfs.data.max_object_size)}.
            </p>
            <pre className="cmd">{`git lfs install
git lfs track "*.bin"        # then commit .gitattributes
git push                     # objects go to ${lfs.data.endpoint}`}</pre>
            {lfs.data.objects.length ? (
              <table className="lfs-table">
                <thead>
                  <tr>
                    <th>Object</th>
                    <th>Size</th>
                    <th>Uploaded</th>
                  </tr>
                </thead>
                <tbody>
                  {lfs.data.objects.map((o) => (
                    <tr key={o.oid}>
                      <td>
                        <code>{o.oid.slice(0, 16)}…</code>
                      </td>
                      <td>{humanSize(o.size)}</td>
                      <td>
                        <RelativeTime datetime={o.created_at} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : null}
            <h3 style={{ margin: 0, fontSize: 16 }}>File locks</h3>
            <div className="key-list">
              {lfs.data.locks.map((l) => (
                <div key={l.id} className="item" style={{ alignItems: 'center' }}>
                  <LockIcon className="muted" />
                  <div className="body">
                    <code>{l.path}</code>{' '}
                    <span className="muted small">
                      locked by {l.owner} <RelativeTime datetime={l.locked_at} />
                    </span>
                  </div>
                  <Button size="small" variant="danger" onClick={() => confirm(`Force-unlock ${l.path}?`) && unlock.mutate(l.id)}>
                    Force unlock
                  </Button>
                </div>
              ))}
              {lfs.data.locks.length === 0 ? <p className="muted" style={{ padding: 16 }}>No files are locked.</p> : null}
            </div>
          </>
        ) : null}
      </div>
    </div>
  );
}
