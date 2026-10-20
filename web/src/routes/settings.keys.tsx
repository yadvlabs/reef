import { Button, FormControl, TextInput, Textarea, RelativeTime } from '@primer/react';
import { KeyIcon, ShieldLockIcon } from '@primer/octicons-react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { ErrorFlash } from '../components/Loading';
import { api, type GpgKey, type SshKey, type SshSigningKey } from '../lib/api';

export const Route = createFileRoute('/settings/keys')({ component: Keys });

function Keys() {
  return (
    <div>
      <SshKeys />
      <GpgKeys />
      <SigningKeys />
    </div>
  );
}

function SectionHead({ title, action, onAction }: { title: string; action: string; onAction: () => void }) {
  return (
    <h2 className="section-title" style={{ display: 'flex', alignItems: 'center' }}>
      {title}
      <span style={{ flex: 1 }} />
      <Button variant="primary" size="small" onClick={onAction}>
        {action}
      </Button>
    </h2>
  );
}

function SshKeys() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['user', 'keys'], queryFn: () => api<SshKey[]>('/user/keys') });
  const [error, setError] = useState<unknown>(null);
  const [adding, setAdding] = useState(false);
  async function add(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setError(null);
    const f = new FormData(e.currentTarget);
    try {
      await api('/user/keys', { method: 'POST', json: { title: f.get('title'), key: f.get('key') } });
      qc.invalidateQueries({ queryKey: ['user', 'keys'] });
      setAdding(false);
    } catch (err) {
      setError(err);
    }
  }
  async function remove(k: SshKey) {
    if (!confirm(`Delete the key "${k.title}"?`)) return;
    await api(`/user/keys/${k.id}`, { method: 'DELETE' });
    qc.invalidateQueries({ queryKey: ['user', 'keys'] });
  }
  return (
    <section style={{ marginBottom: 40 }}>
      <SectionHead title="SSH keys" action="New SSH key" onAction={() => setAdding(true)} />
      {error ? <ErrorFlash error={error} /> : null}
      {adding ? (
        <form className="form-stack" onSubmit={add} style={{ marginBottom: 24 }}>
          <FormControl>
            <FormControl.Label>Title</FormControl.Label>
            <TextInput name="title" block placeholder="Work laptop" />
          </FormControl>
          <FormControl required>
            <FormControl.Label>Key</FormControl.Label>
            <Textarea name="key" block rows={4} placeholder="Begins with 'ssh-ed25519', 'ssh-rsa', or 'ecdsa-sha2-nistp256'" />
          </FormControl>
          <div style={{ display: 'flex', gap: 8 }}>
            <Button type="submit" variant="primary">
              Add SSH key
            </Button>
            <Button onClick={() => setAdding(false)}>Cancel</Button>
          </div>
        </form>
      ) : null}
      <div className="key-list">
        {q.data?.map((k) => (
          <div key={k.id} className="item">
            <KeyIcon size={24} className="muted" />
            <div className="body">
              <strong>{k.title}</strong>
              <div className="monospace small muted" style={{ wordBreak: 'break-all' }}>
                {k.key.split(' ').slice(0, 2).join(' ').slice(0, 70)}…
              </div>
              <div className="small muted">
                Added <RelativeTime datetime={k.created_at} />
              </div>
            </div>
            <Button variant="danger" size="small" onClick={() => remove(k)}>
              Delete
            </Button>
          </div>
        ))}
        {q.data?.length === 0 ? <p className="muted">There are no SSH keys associated with your account.</p> : null}
      </div>
      <p className="muted small" style={{ marginTop: 16 }}>
        Keys authenticate git over SSH. Clone with the SSH URL shown in the Code menu of any repository.
      </p>
    </section>
  );
}

function GpgKeys() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['user', 'gpg_keys'], queryFn: () => api<GpgKey[]>('/user/gpg_keys') });
  const [error, setError] = useState<unknown>(null);
  const [adding, setAdding] = useState(false);
  async function add(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setError(null);
    const f = new FormData(e.currentTarget);
    try {
      await api('/user/gpg_keys', { method: 'POST', json: { name: f.get('name'), armored_public_key: f.get('key') } });
      qc.invalidateQueries({ queryKey: ['user', 'gpg_keys'] });
      setAdding(false);
    } catch (err) {
      setError(err);
    }
  }
  async function remove(k: GpgKey) {
    if (!confirm(`Delete GPG key ${k.key_id}? Commits signed with it will show as unverified.`)) return;
    await api(`/user/gpg_keys/${k.id}`, { method: 'DELETE' });
    qc.invalidateQueries({ queryKey: ['user', 'gpg_keys'] });
  }
  return (
    <section style={{ marginBottom: 40 }}>
      <SectionHead title="GPG keys" action="New GPG key" onAction={() => setAdding(true)} />
      {error ? <ErrorFlash error={error} /> : null}
      {adding ? (
        <form className="form-stack" onSubmit={add} style={{ marginBottom: 24 }}>
          <FormControl>
            <FormControl.Label>Title</FormControl.Label>
            <TextInput name="name" block placeholder="Work key" />
          </FormControl>
          <FormControl required>
            <FormControl.Label>Key</FormControl.Label>
            <Textarea name="key" block rows={8} placeholder="Begins with '-----BEGIN PGP PUBLIC KEY BLOCK-----'" />
            <FormControl.Caption>Export it with `gpg --armor --export &lt;key id&gt;`.</FormControl.Caption>
          </FormControl>
          <div style={{ display: 'flex', gap: 8 }}>
            <Button type="submit" variant="primary">
              Add GPG key
            </Button>
            <Button onClick={() => setAdding(false)}>Cancel</Button>
          </div>
        </form>
      ) : null}
      <div className="key-list">
        {q.data?.map((k) => (
          <div key={k.id} className="item">
            <ShieldLockIcon size={24} className="muted" />
            <div className="body">
              <div className="small muted">
                Email address{k.emails.length === 1 ? '' : 'es'}: {k.emails.length ? k.emails.map((e) => e.email).join(', ') : 'none in key'}
              </div>
              <div className="gpg-key-line">
                Key ID: {k.key_id}
                {k.subkeys.length ? ` · Subkeys: ${k.subkeys.map((sk) => sk.key_id).join(', ')}` : ''}
              </div>
              <div className="small muted">
                Added <RelativeTime datetime={k.created_at} />
                {k.expires_at ? (
                  <>
                    {' '}
                    · Expires <RelativeTime datetime={k.expires_at} />
                  </>
                ) : (
                  ' · Never expires'
                )}
                {' · '}
                {[k.can_sign && 'sign', k.can_certify && 'certify', k.can_encrypt_comms && 'encrypt'].filter(Boolean).join(', ') || 'no capabilities'}
              </div>
            </div>
            <Button variant="danger" size="small" onClick={() => remove(k)}>
              Delete
            </Button>
          </div>
        ))}
        {q.data?.length === 0 ? <p className="muted">There are no GPG keys associated with your account.</p> : null}
      </div>
      <p className="muted small" style={{ marginTop: 16 }}>
        Commits signed with a registered key whose user id matches your committer email show a Verified badge.
      </p>
    </section>
  );
}

function SigningKeys() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['user', 'ssh_signing_keys'], queryFn: () => api<SshSigningKey[]>('/user/ssh_signing_keys') });
  const [error, setError] = useState<unknown>(null);
  const [adding, setAdding] = useState(false);
  async function add(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setError(null);
    const f = new FormData(e.currentTarget);
    try {
      await api('/user/ssh_signing_keys', { method: 'POST', json: { title: f.get('title'), key: f.get('key') } });
      qc.invalidateQueries({ queryKey: ['user', 'ssh_signing_keys'] });
      setAdding(false);
    } catch (err) {
      setError(err);
    }
  }
  async function remove(k: SshSigningKey) {
    if (!confirm(`Delete the signing key "${k.title}"?`)) return;
    await api(`/user/ssh_signing_keys/${k.id}`, { method: 'DELETE' });
    qc.invalidateQueries({ queryKey: ['user', 'ssh_signing_keys'] });
  }
  return (
    <section>
      <SectionHead title="SSH signing keys" action="New signing key" onAction={() => setAdding(true)} />
      {error ? <ErrorFlash error={error} /> : null}
      {adding ? (
        <form className="form-stack" onSubmit={add} style={{ marginBottom: 24 }}>
          <FormControl>
            <FormControl.Label>Title</FormControl.Label>
            <TextInput name="title" block placeholder="Laptop signing key" />
          </FormControl>
          <FormControl required>
            <FormControl.Label>Key</FormControl.Label>
            <Textarea name="key" block rows={4} placeholder="Begins with 'ssh-ed25519', 'ssh-rsa', or 'ecdsa-sha2-nistp256'" />
            <FormControl.Caption>Configure git with `gpg.format=ssh` and `user.signingkey` pointing at this key to sign commits.</FormControl.Caption>
          </FormControl>
          <div style={{ display: 'flex', gap: 8 }}>
            <Button type="submit" variant="primary">
              Add signing key
            </Button>
            <Button onClick={() => setAdding(false)}>Cancel</Button>
          </div>
        </form>
      ) : null}
      <div className="key-list">
        {q.data?.map((k) => (
          <div key={k.id} className="item">
            <KeyIcon size={24} className="muted" />
            <div className="body">
              <strong>{k.title}</strong>
              <div className="monospace small muted" style={{ wordBreak: 'break-all' }}>
                {k.key.slice(0, 70)}…
              </div>
              <div className="small muted">
                Added <RelativeTime datetime={k.created_at} />
              </div>
            </div>
            <Button variant="danger" size="small" onClick={() => remove(k)}>
              Delete
            </Button>
          </div>
        ))}
        {q.data?.length === 0 ? <p className="muted">There are no SSH signing keys associated with your account.</p> : null}
      </div>
    </section>
  );
}
