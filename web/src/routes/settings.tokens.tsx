import { Button, Checkbox, FormControl, Label, Select, TextInput, RelativeTime } from '@primer/react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { ErrorFlash } from '../components/Loading';
import { reef, type Token } from '../lib/api';

export const Route = createFileRoute('/settings/tokens')({ component: Tokens });

const SCOPES: [string, string][] = [
  ['repo', 'Full control of repositories'],
  ['workflow', 'Update workflows'],
  ['read:org', 'Read organization membership'],
  ['admin:org', 'Manage organizations'],
  ['admin:public_key', 'Manage SSH keys'],
  ['delete_repo', 'Delete repositories'],
  ['user', 'Read and update profile'],
];

function Tokens() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['user', 'tokens'], queryFn: () => reef<Token[]>('/tokens') });
  const [error, setError] = useState<unknown>(null);
  const [creating, setCreating] = useState(false);
  const [minted, setMinted] = useState<Token | null>(null);
  async function create(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setError(null);
    const f = new FormData(e.currentTarget);
    const scopes = SCOPES.map(([s]) => s).filter((s) => f.get(`scope:${s}`) === 'on');
    try {
      const t = await reef<Token>('/tokens', { method: 'POST', json: { name: f.get('name'), scopes, expires_in_days: Number(f.get('expires')) || null } });
      setMinted(t);
      setCreating(false);
      qc.invalidateQueries({ queryKey: ['user', 'tokens'] });
    } catch (err) {
      setError(err);
    }
  }
  async function remove(t: Token) {
    if (!confirm(`Delete the token "${t.name}"? Anything using it will stop working.`)) return;
    await reef(`/tokens/${t.id}`, { method: 'DELETE' });
    qc.invalidateQueries({ queryKey: ['user', 'tokens'] });
  }
  return (
    <div>
      <h2 className="section-title" style={{ display: 'flex', alignItems: 'center' }}>
        Personal access tokens
        <span style={{ flex: 1 }} />
        <Button variant="primary" size="small" onClick={() => { setCreating(true); setMinted(null); }}>
          Generate new token
        </Button>
      </h2>
      {error ? <ErrorFlash error={error} /> : null}
      {minted?.token ? (
        <div style={{ marginBottom: 24 }}>
          <p>Make sure to copy your personal access token now. You won't be able to see it again!</p>
          <div className="token-reveal">
            {minted.token}
            <Button size="small" onClick={() => navigator.clipboard.writeText(minted.token!)}>
              Copy
            </Button>
          </div>
          <p className="muted small" style={{ marginTop: 8 }}>
            Use it as the password for git over HTTPS, or with <code>gh auth login --with-token</code>.
          </p>
        </div>
      ) : null}
      {creating ? (
        <form className="form-stack" onSubmit={create} style={{ marginBottom: 24 }}>
          <FormControl required>
            <FormControl.Label>Note</FormControl.Label>
            <TextInput name="name" block placeholder="What's this token for?" />
          </FormControl>
          <FormControl>
            <FormControl.Label>Expiration</FormControl.Label>
            <Select name="expires" defaultValue="90">
              <Select.Option value="7">7 days</Select.Option>
              <Select.Option value="30">30 days</Select.Option>
              <Select.Option value="90">90 days</Select.Option>
              <Select.Option value="365">1 year</Select.Option>
              <Select.Option value="0">No expiration</Select.Option>
            </Select>
          </FormControl>
          <div>
            <div style={{ fontWeight: 600, marginBottom: 8 }}>Select scopes</div>
            {SCOPES.map(([s, d]) => (
              <FormControl key={s}>
                <Checkbox name={`scope:${s}`} defaultChecked={s === 'repo' || s === 'read:org' || s === 'workflow'} />
                <FormControl.Label>
                  <code>{s}</code>
                </FormControl.Label>
                <FormControl.Caption>{d}</FormControl.Caption>
              </FormControl>
            ))}
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <Button type="submit" variant="primary">
              Generate token
            </Button>
            <Button onClick={() => setCreating(false)}>Cancel</Button>
          </div>
        </form>
      ) : null}
      <div className="key-list">
        {q.data?.map((t) => (
          <div key={t.id} className="item">
            <div className="body">
              <strong>{t.name}</strong> <span className="muted monospace small">…{t.token_last4}</span>
              {t.kind === 'oauth' ? <Label style={{ marginLeft: 8 }}>OAuth</Label> : null}
              <div className="small muted">
                {t.scopes.join(', ') || 'no scopes'} · created <RelativeTime datetime={t.created_at} />
                {t.last_used_at ? <> · last used <RelativeTime datetime={t.last_used_at} /></> : ' · never used'}
                {t.expires_at ? <> · expires <RelativeTime datetime={t.expires_at} /></> : null}
              </div>
            </div>
            <Button variant="danger" size="small" onClick={() => remove(t)}>
              Delete
            </Button>
          </div>
        ))}
        {q.data?.length === 0 ? <p className="muted">No tokens yet.</p> : null}
      </div>
    </div>
  );
}
