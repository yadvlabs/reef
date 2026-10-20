import { Button, FormControl, TextInput } from '@primer/react';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash } from '../components/Loading';
import { Logo } from '../components/Logo';
import { reef } from '../lib/api';
import { useRefreshSession, useSession } from '../lib/session';

export const Route = createFileRoute('/join')({ component: Join });

function Join() {
  const { instance } = useSession();
  const refresh = useRefreshSession();
  const navigate = useNavigate();
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const f = new FormData(e.currentTarget);
    try {
      await reef('/signup', { method: 'POST', json: { login: f.get('login'), email: f.get('email'), password: f.get('password'), name: f.get('name') } });
      await refresh();
      navigate({ to: '/' });
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <AppHeader />
      <div className="center-card">
        <div style={{ textAlign: 'center', marginBottom: 24 }}>
          <Logo size={48} />
        </div>
        <h1>Create your account</h1>
        {error ? <div style={{ marginBottom: 16 }}><ErrorFlash error={error} /></div> : null}
        <form className="card form-stack" onSubmit={submit}>
          <FormControl required>
            <FormControl.Label>Username</FormControl.Label>
            <TextInput name="login" block autoFocus autoComplete="username" />
            <FormControl.Caption>Letters, numbers, and single hyphens.</FormControl.Caption>
          </FormControl>
          <FormControl required>
            <FormControl.Label>Email address</FormControl.Label>
            <TextInput name="email" type="email" block autoComplete="email" />
          </FormControl>
          <FormControl>
            <FormControl.Label>Full name</FormControl.Label>
            <TextInput name="name" block autoComplete="name" />
          </FormControl>
          <FormControl required>
            <FormControl.Label>Password</FormControl.Label>
            <TextInput name="password" type="password" block autoComplete="new-password" />
            <FormControl.Caption>At least 8 characters.</FormControl.Caption>
          </FormControl>
          <Button type="submit" variant="primary" block loading={busy}>
            Create account
          </Button>
        </form>
        <p className="after">
          Already have an account? <Link to="/login">Sign in</Link>.
        </p>
        {instance ? <p className="muted small" style={{ textAlign: 'center' }}>The first account becomes the site administrator.</p> : null}
      </div>
    </>
  );
}
