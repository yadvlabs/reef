import { Button, FormControl, TextInput } from '@primer/react';
import { CopyIcon, TrashIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { ErrorFlash } from '../components/Loading';
import { SettingsNav } from '../components/SettingsNav';
import { api } from '../lib/api';
import { runnersQuery } from '../lib/actions';
import { useSession } from '../lib/session';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/settings/actions')({ component: ActionsSettings });

interface Secret {
  name: string;
  updated_at: string;
}
interface Variable {
  name: string;
  value: string;
}

function ActionsSettings() {
  const repo = useRepo();
  const { instance } = useSession();
  const qc = useQueryClient();
  const full = repo.full_name;
  const runners = useQuery(runnersQuery(full));
  const secrets = useQuery({ queryKey: ['actions', full.toLowerCase(), 'secrets'], queryFn: () => api<{ secrets: Secret[] }>(`/repos/${full}/actions/secrets`) });
  const variables = useQuery({ queryKey: ['actions', full.toLowerCase(), 'variables'], queryFn: () => api<{ variables: Variable[] }>(`/repos/${full}/actions/variables`) });
  const [token, setToken] = useState<{ token: string; expires_at: string } | null>(null);
  const [error, setError] = useState<unknown>(null);
  const invalidate = () => qc.invalidateQueries({ queryKey: ['actions', full.toLowerCase()] });
  const mint = useMutation({ mutationFn: () => api<{ token: string; expires_at: string }>(`/repos/${full}/actions/runners/registration-token`, { method: 'POST' }), onSuccess: setToken, onError: setError });
  const removeRunner = useMutation({ mutationFn: (id: number) => api(`/repos/${full}/actions/runners/${id}`, { method: 'DELETE' }), onSuccess: invalidate, onError: setError });
  const delSecret = useMutation({ mutationFn: (name: string) => api(`/repos/${full}/actions/secrets/${name}`, { method: 'DELETE' }), onSuccess: invalidate, onError: setError });
  const setVar = useMutation({ mutationFn: ({ name, value }: { name: string; value: string }) => api(`/repos/${full}/actions/variables`, { method: 'POST', json: { name, value } }), onSuccess: invalidate, onError: setError });
  const delVar = useMutation({ mutationFn: (name: string) => api(`/repos/${full}/actions/variables/${name}`, { method: 'DELETE' }), onSuccess: invalidate, onError: setError });
  const [varName, setVarName] = useState('');
  const [varValue, setVarValue] = useState('');
  const site = instance?.site_url ?? location.origin;
  return (
    <div className="settings-layout">
      <SettingsNav repo={repo} current="actions" />
      <div className="form-stack">
        {error ? <ErrorFlash error={error} /> : null}
        <h2 className="section-title">Runners</h2>
        <p className="muted small">Runners execute the jobs of this repository's workflows. Any stock <code>actions/runner</code> release works; register it with a token below. Organization runners also serve this repository.</p>
        <div className="key-list">
          {runners.data?.runners.map((r) => (
            <div key={r.id} className="item" style={{ alignItems: 'center' }}>
              <span className={`runner-dot ${r.status}`} />
              <div className="body">
                <strong>{r.name}</strong> <span className="muted small">{r.os}{r.version ? ` · v${r.version}` : ''} · {r.busy ? 'Active' : r.status === 'online' ? 'Idle' : 'Offline'}</span>
                <div style={{ display: 'flex', gap: 4, marginTop: 4, flexWrap: 'wrap' }}>
                  {r.labels.map((l) => (
                    <span key={l.id} className="visibility-pill">
                      {l.name}
                    </span>
                  ))}
                </div>
              </div>
              <Button size="small" variant="danger" leadingVisual={TrashIcon} onClick={() => confirm(`Remove runner ${r.name}?`) && removeRunner.mutate(r.id)}>
                Remove
              </Button>
            </div>
          ))}
          {runners.data?.runners.length === 0 ? <p className="muted" style={{ padding: 16 }}>No runners registered yet.</p> : null}
        </div>
        <div className="card form-stack">
          <h3 style={{ margin: 0, fontSize: 16 }}>Add a new runner</h3>
          <Button onClick={() => mint.mutate()} loading={mint.isPending} style={{ alignSelf: 'flex-start' }}>
            Generate registration token
          </Button>
          {token ? (
            <>
              <p className="muted small" style={{ margin: 0 }}>The token expires <RelTime iso={token.expires_at} />. Download a runner and run:</p>
              <pre className="cmd">{`# download and unpack the runner (see gh api /repos/${full}/actions/runners/downloads)
./config.sh --url ${site}/${full} --token ${token.token} --labels self-hosted,reef
./run.sh`}</pre>
              <Button size="small" leadingVisual={CopyIcon} onClick={() => navigator.clipboard.writeText(token.token)} style={{ alignSelf: 'flex-start' }}>
                Copy token
              </Button>
              <p className="muted small" style={{ margin: 0 }}>
                The runner derives the API address from the host of <code>--url</code> without a port, so the site must be reachable on port 80 or 443 (set <code>REEF_SITE_URL</code> accordingly).
              </p>
            </>
          ) : null}
        </div>

        <h2 className="section-title">Secrets</h2>
        <p className="muted small">Secrets are encrypted at rest and exposed to workflows as <code>{'${{ secrets.NAME }}'}</code>. Set them with <code>gh secret set NAME --repo {full}</code>.</p>
        <div className="key-list">
          {secrets.data?.secrets.map((sec) => (
            <div key={sec.name} className="item" style={{ alignItems: 'center' }}>
              <div className="body">
                <code>{sec.name}</code> <span className="muted small">updated <RelTime iso={sec.updated_at} /></span>
              </div>
              <Button size="small" variant="danger" onClick={() => confirm(`Delete secret ${sec.name}?`) && delSecret.mutate(sec.name)}>
                Delete
              </Button>
            </div>
          ))}
          {secrets.data?.secrets.length === 0 ? <p className="muted" style={{ padding: 16 }}>No secrets.</p> : null}
        </div>

        <h2 className="section-title">Variables</h2>
        <div className="key-list">
          {variables.data?.variables.map((v) => (
            <div key={v.name} className="item" style={{ alignItems: 'center' }}>
              <div className="body">
                <code>{v.name}</code> <span className="muted small">{v.value}</span>
              </div>
              <Button size="small" variant="danger" onClick={() => delVar.mutate(v.name)}>
                Delete
              </Button>
            </div>
          ))}
        </div>
        <form
          style={{ display: 'flex', gap: 8, alignItems: 'flex-end' }}
          onSubmit={(e) => {
            e.preventDefault();
            if (varName.trim()) setVar.mutate({ name: varName.trim(), value: varValue }, { onSuccess: () => { setVarName(''); setVarValue(''); } });
          }}
        >
          <FormControl>
            <FormControl.Label>Name</FormControl.Label>
            <TextInput value={varName} onChange={(e) => setVarName(e.target.value)} placeholder="MY_VAR" />
          </FormControl>
          <FormControl>
            <FormControl.Label>Value</FormControl.Label>
            <TextInput value={varValue} onChange={(e) => setVarValue(e.target.value)} />
          </FormControl>
          <Button type="submit" loading={setVar.isPending}>
            Add variable
          </Button>
        </form>
      </div>
    </div>
  );
}

function RelTime({ iso }: { iso: string }) {
  return <span>{new Date(iso).toLocaleString()}</span>;
}
