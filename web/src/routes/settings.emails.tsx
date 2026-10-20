import { Button, FormControl, Label, TextInput } from '@primer/react';
import { MailIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { ErrorFlash } from '../components/Loading';
import { api, type UserEmail } from '../lib/api';

export const Route = createFileRoute('/settings/emails')({ component: Emails });

function Emails() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ['user', 'emails'], queryFn: () => api<UserEmail[]>('/user/emails') });
  const [email, setEmail] = useState('');
  const [error, setError] = useState<unknown>(null);
  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ['user', 'emails'] });
    qc.invalidateQueries({ queryKey: ['session'] });
  };
  const add = useMutation({
    mutationFn: (e: string) => api('/user/emails', { method: 'POST', json: { emails: [e] } }),
    onSuccess: () => {
      setEmail('');
      setError(null);
      invalidate();
    },
    onError: setError,
  });
  const remove = useMutation({ mutationFn: (e: string) => api('/user/emails', { method: 'DELETE', json: { emails: [e] } }), onSuccess: invalidate, onError: setError });
  const primary = useMutation({ mutationFn: (e: string) => api('/user/emails/primary', { method: 'PATCH', json: { email: e } }), onSuccess: invalidate, onError: setError });
  const visibility = useMutation({ mutationFn: (v: string) => api('/user/email/visibility', { method: 'PATCH', json: { visibility: v } }), onSuccess: invalidate, onError: setError });
  const list = q.data ?? [];
  const isPublic = list.some((e) => e.visibility === 'public');
  return (
    <div>
      <h2 className="section-title">Emails</h2>
      {error ? <ErrorFlash error={error} /> : null}
      <div className="key-list email-list">
        {list.map((e) => (
          <div key={e.email} className="item" style={{ alignItems: 'center' }}>
            <MailIcon size={20} className="muted" />
            <div className="body">
              <strong>
                {e.email}
                {e.primary ? <Label variant="accent">Primary</Label> : null}
                {e.visibility === 'public' ? <Label>Public</Label> : null}
              </strong>
              <div className="small muted">
                {e.primary ? 'This email will be used for account-related notifications and can also be used for password resets.' : 'Commits authored with this address are attributed to your account.'}
              </div>
            </div>
            {!e.primary ? (
              <div style={{ display: 'flex', gap: 6 }}>
                <Button size="small" onClick={() => primary.mutate(e.email)}>
                  Set as primary
                </Button>
                <Button size="small" variant="danger" onClick={() => confirm(`Remove ${e.email} from your account?`) && remove.mutate(e.email)}>
                  Remove
                </Button>
              </div>
            ) : null}
          </div>
        ))}
        {q.data && list.length === 0 ? <p className="muted">No email addresses on this account.</p> : null}
      </div>
      <form
        className="form-stack"
        style={{ marginTop: 24, maxWidth: 480 }}
        onSubmit={(e) => {
          e.preventDefault();
          if (email.trim()) add.mutate(email.trim());
        }}
      >
        <h3 style={{ fontSize: 16, margin: '0 0 8px' }}>Add email address</h3>
        <FormControl required>
          <FormControl.Label>Email address</FormControl.Label>
          <TextInput block type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="you@example.com" />
          <FormControl.Caption>Commits made with any of your addresses show your profile. Address verification by mail is not enabled on this instance.</FormControl.Caption>
        </FormControl>
        <div>
          <Button type="submit" variant="primary" disabled={!email.trim() || add.isPending}>
            Add
          </Button>
        </div>
      </form>
      <h3 style={{ fontSize: 16, margin: '32px 0 8px' }}>Primary email visibility</h3>
      <p className="muted small">Choose whether your primary email address appears on your public profile and in the API.</p>
      <div style={{ display: 'flex', gap: 8 }}>
        <Button size="small" variant={isPublic ? 'primary' : 'default'} onClick={() => visibility.mutate('public')}>
          Public
        </Button>
        <Button size="small" variant={!isPublic ? 'primary' : 'default'} onClick={() => visibility.mutate('private')}>
          Private
        </Button>
      </div>
    </div>
  );
}
