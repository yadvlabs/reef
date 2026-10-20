import { Button, FormControl, TextInput } from '@primer/react';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading } from '../components/Loading';
import { reef, type User } from '../lib/api';
import { useRefreshSession, useSession } from '../lib/session';

export const Route = createFileRoute('/organizations/new')({ component: NewOrg });

function NewOrg() {
  const { user, loading } = useSession();
  const refresh = useRefreshSession();
  const navigate = useNavigate();
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  if (loading) return <Loading />;
  if (!user) {
    navigate({ to: '/login', search: { return_to: '/organizations/new' } });
    return null;
  }
  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const f = new FormData(e.currentTarget);
    try {
      const org = await reef<User>('/orgs', { method: 'POST', json: { login: f.get('login'), name: f.get('name') } });
      await refresh();
      navigate({ to: '/$owner', params: { owner: org.login } });
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <AppHeader />
      <div className="page form">
        <h1 className="section-title">Set up your organization</h1>
        {error ? <div style={{ marginBottom: 16 }}><ErrorFlash error={error} /></div> : null}
        <form className="form-stack" onSubmit={submit}>
          <FormControl required>
            <FormControl.Label>Organization account name</FormControl.Label>
            <TextInput name="login" autoFocus />
            <FormControl.Caption>This will be the organization's URL: {location.origin}/name</FormControl.Caption>
          </FormControl>
          <FormControl>
            <FormControl.Label>Display name</FormControl.Label>
            <TextInput name="name" />
          </FormControl>
          <div>
            <Button type="submit" variant="primary" loading={busy}>
              Create organization
            </Button>
          </div>
        </form>
      </div>
    </>
  );
}
