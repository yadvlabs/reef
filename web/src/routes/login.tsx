import { Button, FormControl, TextInput } from '@primer/react';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash } from '../components/Loading';
import { Wordmark } from '../components/Logo';
import { reef } from '../lib/api';
import { useRefreshSession, useSession } from '../lib/session';

export const Route = createFileRoute('/login')({
  validateSearch: (s: Record<string, unknown>): { return_to?: string; error?: string; link?: string; hint?: string } => ({
    ...(typeof s.return_to === 'string' ? { return_to: s.return_to } : {}),
    ...(typeof s.error === 'string' ? { error: s.error } : {}),
    ...(typeof s.link === 'string' ? { link: s.link } : {}),
    ...(typeof s.hint === 'string' ? { hint: s.hint } : {}),
  }),
  component: Login,
});

function Login() {
  const { return_to, error: urlError, link, hint } = Route.useSearch();
  const { instance } = useSession();
  const refresh = useRefreshSession();
  const navigate = useNavigate();
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const pending = useQuery({ queryKey: ['pending-link', link], queryFn: () => reef<{ provider: string; username: string | null; email: string | null; return_to: string | null }>(`/session/link?link=${encodeURIComponent(link!)}`), enabled: !!link, retry: false });

  async function createSeparate() {
    setBusy(true);
    setError(null);
    try {
      await reef('/session/link-create', { method: 'POST', json: { link } });
      await refresh();
      navigate({ to: pending.data?.return_to ?? return_to ?? '/' });
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  }

  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const f = new FormData(e.currentTarget);
    try {
      await reef('/session', { method: 'POST', json: { login: f.get('login'), password: f.get('password'), link: link ?? undefined } });
      await refresh();
      navigate({ to: pending.data?.return_to ?? return_to ?? '/' });
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
        <div style={{ textAlign: 'center', marginBottom: 16 }}>
          <Wordmark height={64} />
          <div className="muted" style={{ marginTop: 4, fontSize: 14, letterSpacing: 1, textTransform: 'uppercase' }}>{instance?.name ?? 'Reef'}</div>
        </div>
        <h1>{link ? 'Link your identity' : `Sign in to ${instance?.name ?? 'Reef'}`}</h1>
        {link ? (
          <div className="card form-stack" style={{ marginBottom: 16 }}>
            {pending.isError ? (
              <ErrorFlash error={pending.error} />
            ) : (
              <>
                <p style={{ margin: 0 }}>
                  Your {pending.data?.provider ?? instance?.oidc?.name ?? 'identity provider'} identity <strong>{pending.data?.username ?? pending.data?.email ?? ''}</strong> is not linked to a {instance?.name ?? 'Reef'} account yet.
                  {hint ? <> An account named <strong>{hint}</strong> already exists. Sign in to it below to link them.</> : ' Sign in to the account you want to link it to.'}
                </p>
                <Button variant="default" onClick={createSeparate} loading={busy} block>
                  Create a separate account instead
                </Button>
              </>
            )}
          </div>
        ) : null}
        {error ? <div style={{ marginBottom: 16 }}><ErrorFlash error={error} /></div> : null}
        {urlError ? <div style={{ marginBottom: 16 }}><ErrorFlash error={urlError === 'no-account' ? 'No Reef account is linked to that identity, and automatic account creation is off. Ask an administrator.' : urlError === 'email-taken' ? 'A Reef account already uses that email address. Sign in with your password, then link the identity under Settings.' : urlError} /></div> : null}
        {instance?.oidc && !link ? (
          <div className="card form-stack" style={{ marginBottom: 16 }}>
            <Button as="a" {...({ href: `${instance.oidc.login_url}${return_to ? `?return_to=${encodeURIComponent(return_to)}` : ''}` } as object)} variant="primary" block>
              Sign in with {instance.oidc.name}
            </Button>
          </div>
        ) : null}
        {instance?.password_login === false ? null : (
        <form className="card form-stack" onSubmit={submit}>
          <FormControl required>
            <FormControl.Label>Username or email address</FormControl.Label>
            <TextInput name="login" block autoFocus autoComplete="username" defaultValue={hint ?? ''} />
          </FormControl>
          <FormControl required>
            <FormControl.Label>Password</FormControl.Label>
            <TextInput name="password" type="password" block autoComplete="current-password" />
          </FormControl>
          <Button type="submit" variant="primary" block loading={busy}>
            Sign in
          </Button>
        </form>
        )}
        {instance?.allow_signup ? (
          <p className="after">
            New to {instance.name}? <Link to="/join">Create an account</Link>.
          </p>
        ) : null}
      </div>
    </>
  );
}
