import { Button, FormControl, TextInput } from '@primer/react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { ErrorFlash } from '../components/Loading';
import { reef } from '../lib/api';

export const Route = createFileRoute('/settings/security')({ component: Security });

interface Identities {
  has_password: boolean;
  identities: { issuer: string; subject: string; email: string | null }[];
}

function Security() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['user', 'identities'], queryFn: () => reef<Identities>('/identities') });
  const [error, setError] = useState<unknown>(null);
  const [done, setDone] = useState(false);
  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setError(null);
    setDone(false);
    const f = new FormData(e.currentTarget);
    if (f.get('new_password') !== f.get('confirm')) {
      setError(new Error('The new passwords do not match'));
      return;
    }
    try {
      await reef('/password', { method: 'POST', json: { current_password: f.get('current_password'), new_password: f.get('new_password') } });
      setDone(true);
      (e.target as HTMLFormElement).reset();
      qc.invalidateQueries({ queryKey: ['user', 'identities'] });
    } catch (err) {
      setError(err);
    }
  }
  return (
    <div>
      <h2 className="section-title">{q.data?.has_password ? 'Change password' : 'Set a password'}</h2>
      {error ? <ErrorFlash error={error} /> : null}
      <form className="form-stack" onSubmit={submit} style={{ maxWidth: 420 }}>
        {q.data?.has_password ? (
          <FormControl required>
            <FormControl.Label>Current password</FormControl.Label>
            <TextInput name="current_password" type="password" block autoComplete="current-password" />
          </FormControl>
        ) : (
          <p className="muted">Your account signs in through an identity provider. Setting a password also lets you use HTTPS git with a password and the CLI device flow.</p>
        )}
        <FormControl required>
          <FormControl.Label>New password</FormControl.Label>
          <TextInput name="new_password" type="password" block autoComplete="new-password" />
          <FormControl.Caption>At least 8 characters.</FormControl.Caption>
        </FormControl>
        <FormControl required>
          <FormControl.Label>Confirm new password</FormControl.Label>
          <TextInput name="confirm" type="password" block autoComplete="new-password" />
        </FormControl>
        <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
          <Button type="submit" variant="primary">
            Update password
          </Button>
          {done ? <span className="muted">Password updated. Other sessions were signed out.</span> : null}
        </div>
      </form>
      <h2 className="section-title" style={{ marginTop: 32 }}>Linked identities</h2>
      {q.data?.identities.length ? (
        <div className="key-list">
          {q.data.identities.map((i) => (
            <div key={i.issuer + i.subject} className="item">
              <div className="body">
                <strong>{i.issuer}</strong>
                <div className="muted small">
                  subject {i.subject}
                  {i.email ? ` · ${i.email}` : ''}
                </div>
              </div>
            </div>
          ))}
        </div>
      ) : (
        <p className="muted">No identity providers are linked to this account.</p>
      )}
    </div>
  );
}
