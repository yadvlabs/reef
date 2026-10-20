import { Button, FormControl, TextInput, Flash, Label } from '@primer/react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading } from '../components/Loading';
import { reef } from '../lib/api';
import { useSession } from '../lib/session';

interface Grant {
  user_code: string;
  client_id: string;
  client_name: string;
  scopes: string[];
  status: string;
  expires_at: string;
}

export const Route = createFileRoute('/login_/device')({ component: Device });

function Device() {
  const { user, loading } = useSession();
  const navigate = useNavigate();
  const [grant, setGrant] = useState<Grant | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [done, setDone] = useState<'approved' | 'denied' | null>(null);
  const [busy, setBusy] = useState(false);

  if (loading) return <Loading />;
  if (!user) {
    navigate({ to: '/login', search: { return_to: '/login/device' } });
    return null;
  }

  async function lookup(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setError(null);
    const code = String(new FormData(e.currentTarget).get('code') ?? '').trim().toUpperCase();
    try {
      setGrant(await reef<Grant>(`/device?user_code=${encodeURIComponent(code)}`));
    } catch (err) {
      setError(err);
    }
  }

  async function decide(approve: boolean) {
    if (!grant) return;
    setBusy(true);
    try {
      await reef('/device', { method: 'POST', json: { user_code: grant.user_code, approve } });
      setDone(approve ? 'approved' : 'denied');
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <AppHeader />
      <div className="center-card wide">
        <h1>Device activation</h1>
        {error ? <div style={{ marginBottom: 16 }}><ErrorFlash error={error} /></div> : null}
        {done ? (
          <Flash variant={done === 'approved' ? 'success' : 'warning'}>
            {done === 'approved' ? 'Congratulations, you are all set! You may now close this window and return to your device.' : 'The request was denied.'}
          </Flash>
        ) : grant ? (
          <div className="card form-stack">
            <p style={{ margin: 0 }}>
              <strong>{grant.client_name}</strong> wants to access your account as <strong>{user.login}</strong>.
            </p>
            <div>
              <div className="small muted" style={{ marginBottom: 4 }}>Requested permissions</div>
              {grant.scopes.length === 0 ? <Label>default</Label> : grant.scopes.map((s) => <Label key={s} style={{ marginRight: 4 }}>{s}</Label>)}
            </div>
            <div className="device-code">{grant.user_code}</div>
            {grant.status !== 'pending' ? (
              <Flash variant="warning">This code was already used.</Flash>
            ) : (
              <div style={{ display: 'flex', gap: 8 }}>
                <Button variant="primary" block onClick={() => decide(true)} loading={busy}>
                  Authorize {grant.client_name}
                </Button>
                <Button block onClick={() => decide(false)} disabled={busy}>
                  Cancel
                </Button>
              </div>
            )}
          </div>
        ) : (
          <form className="card form-stack" onSubmit={lookup}>
            <FormControl required>
              <FormControl.Label>Enter the code displayed on your device</FormControl.Label>
              <TextInput name="code" block autoFocus placeholder="XXXX-XXXX" monospace size="large" />
            </FormControl>
            <Button type="submit" variant="primary" block>
              Continue
            </Button>
          </form>
        )}
      </div>
    </>
  );
}
