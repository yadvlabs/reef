import { Button, FormControl, Select, TextInput, Avatar, ActionMenu, ActionList } from '@primer/react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { ErrorFlash, NotFound } from '../components/Loading';
import { SettingsNav } from '../components/SettingsNav';
import { api, type Repo, type User } from '../lib/api';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/settings/collaborators')({ component: Page });

function Page() {
  const repo = useRepo();
  const [error, setError] = useState<unknown>(null);
  if (!repo.permissions?.admin) return <NotFound />;
  return (
    <div className="settings-layout">
      <SettingsNav repo={repo} current="collaborators" />
      <div className="form-stack">
        {error ? <ErrorFlash error={error} /> : null}
        <Collaborators repo={repo} onError={setError} />
      </div>
    </div>
  );
}

type Collab = User & { role_name: string };

function Collaborators({ repo, onError }: { repo: Repo; onError: (e: unknown) => void }) {
  const qc = useQueryClient();
  const key = ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase(), 'collaborators'];
  const q = useQuery({ queryKey: key, queryFn: () => api<Collab[]>(`/repos/${repo.full_name}/collaborators`) });
  const [login, setLogin] = useState('');
  const [perm, setPerm] = useState('write');
  async function add(e: React.FormEvent) {
    e.preventDefault();
    try {
      await api(`/repos/${repo.full_name}/collaborators/${login.trim()}`, { method: 'PUT', json: { permission: perm } });
      setLogin('');
      qc.invalidateQueries({ queryKey: key });
    } catch (err) {
      onError(err);
    }
  }
  async function remove(l: string) {
    if (!confirm(`Remove ${l} from this repository?`)) return;
    try {
      await api(`/repos/${repo.full_name}/collaborators/${l}`, { method: 'DELETE' });
      qc.invalidateQueries({ queryKey: key });
    } catch (err) {
      onError(err);
    }
  }
  return (
    <div>
      <h2 className="section-title">Collaborators</h2>
      <div className="key-list">
        {q.data?.map((c) => (
          <div key={c.id} className="item" style={{ alignItems: 'center' }}>
            <Avatar src={c.avatar_url} size={32} alt="" />
            <div className="body">
              <strong>{c.login}</strong>
              <div className="muted small">{c.role_name}</div>
            </div>
            {c.login !== repo.owner.login ? (
              <ActionMenu>
                <ActionMenu.Button size="small">Role: {c.role_name}</ActionMenu.Button>
                <ActionMenu.Overlay>
                  <ActionList>
                    {['read', 'triage', 'write', 'maintain', 'admin'].map((r) => (
                      <ActionList.Item key={r} onSelect={() => api(`/repos/${repo.full_name}/collaborators/${c.login}`, { method: 'PUT', json: { permission: r } }).then(() => qc.invalidateQueries({ queryKey: key })).catch(onError)}>
                        {r}
                      </ActionList.Item>
                    ))}
                    <ActionList.Divider />
                    <ActionList.Item variant="danger" onSelect={() => remove(c.login)}>
                      Remove
                    </ActionList.Item>
                  </ActionList>
                </ActionMenu.Overlay>
              </ActionMenu>
            ) : null}
          </div>
        ))}
      </div>
      <form onSubmit={add} style={{ display: 'flex', gap: 8, marginTop: 12, alignItems: 'flex-end' }}>
        <FormControl>
          <FormControl.Label>Add people</FormControl.Label>
          <TextInput placeholder="username" value={login} onChange={(e) => setLogin(e.target.value)} />
        </FormControl>
        <FormControl>
          <FormControl.Label>Role</FormControl.Label>
          <Select value={perm} onChange={(e) => setPerm(e.target.value)}>
            {['read', 'triage', 'write', 'maintain', 'admin'].map((r) => (
              <Select.Option key={r} value={r}>
                {r}
              </Select.Option>
            ))}
          </Select>
        </FormControl>
        <Button type="submit" disabled={!login.trim()}>
          Add
        </Button>
      </form>
    </div>
  );
}
