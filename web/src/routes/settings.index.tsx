import { Button, FormControl, TextInput, Textarea } from '@primer/react';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { ErrorFlash } from '../components/Loading';
import { AvatarUpload } from '../components/AvatarUpload';
import { useQueryClient } from '@tanstack/react-query';
import { api, reef } from '../lib/api';
import { useRefreshSession, useSession } from '../lib/session';

export const Route = createFileRoute('/settings/')({ component: ProfileSettings });

function ProfileSettings() {
  const { user } = useSession();
  const refresh = useRefreshSession();
  const qc = useQueryClient();
  const [error, setError] = useState<unknown>(null);
  const [saved, setSaved] = useState(false);
  if (!user) return null;
  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setError(null);
    setSaved(false);
    const f = new FormData(e.currentTarget);
    try {
      await api('/user', { method: 'PATCH', json: { name: f.get('name'), email: f.get('email'), bio: f.get('bio'), company: f.get('company'), location: f.get('location'), blog: f.get('blog') } });
      await refresh();
      setSaved(true);
    } catch (err) {
      setError(err);
    }
  }
  return (
    <div>
      <h2 className="section-title">Public profile</h2>
      {error ? <ErrorFlash error={error} /> : null}
      <div className="profile-form-grid">
      <form className="form-stack" onSubmit={submit}>
        <FormControl>
          <FormControl.Label>Name</FormControl.Label>
          <TextInput name="name" defaultValue={user.name ?? ''} block />
        </FormControl>
        <FormControl>
          <FormControl.Label>Email</FormControl.Label>
          <TextInput name="email" type="email" defaultValue={user.email ?? ''} block />
        </FormControl>
        <FormControl>
          <FormControl.Label>Bio</FormControl.Label>
          <Textarea name="bio" defaultValue={user.bio ?? ''} block rows={3} />
        </FormControl>
        <FormControl>
          <FormControl.Label>Company</FormControl.Label>
          <TextInput name="company" defaultValue={user.company ?? ''} block />
        </FormControl>
        <FormControl>
          <FormControl.Label>Location</FormControl.Label>
          <TextInput name="location" defaultValue={user.location ?? ''} block />
        </FormControl>
        <FormControl>
          <FormControl.Label>Website</FormControl.Label>
          <TextInput name="blog" defaultValue={user.blog ?? ''} block />
        </FormControl>
        <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
          <Button type="submit" variant="primary">
            Update profile
          </Button>
          {saved ? <span className="muted">Saved.</span> : null}
        </div>
      </form>
      <div>
        <h3 style={{ fontSize: 14, margin: '0 0 8px' }}>Profile picture</h3>
        <AvatarUpload avatarUrl={user.avatar_url} endpoint="/user/avatar" onChanged={() => { refresh(); qc.invalidateQueries({ queryKey: ['account'] }); }} />
      </div>
      </div>
      <ChangeUsername current={user.login} onChanged={() => { refresh(); qc.invalidateQueries({ queryKey: ['account'] }); }} />
    </div>
  );
}

function ChangeUsername({ current, onChanged }: { current: string; onChanged: () => void }) {
  const [login, setLogin] = useState(current);
  const [error, setError] = useState<unknown>(null);
  const [done, setDone] = useState(false);
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setDone(false);
    try {
      await reef('/user/login', { method: 'PATCH', json: { login: login.trim() } });
      setDone(true);
      onChanged();
      history.replaceState(null, '', location.pathname);
    } catch (err) {
      setError(err);
    }
  };
  return (
    <form onSubmit={submit} className="form-stack" style={{ marginTop: 32, paddingTop: 24, borderTop: '1px solid var(--borderColor-default)' }}>
      <h2 className="section-title">Change username</h2>
      <p className="muted small" style={{ margin: 0 }}>
        Changing your username updates your profile URL and clone URLs. Links to the old name are not redirected.
      </p>
      {error ? <ErrorFlash error={error} /> : null}
      <FormControl>
        <FormControl.Label>Username</FormControl.Label>
        <TextInput value={login} onChange={(e) => setLogin(e.target.value)} block />
      </FormControl>
      <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
        <Button type="submit" disabled={!login.trim() || login.trim() === current}>
          Change username
        </Button>
        {done ? <span className="muted">Username changed.</span> : null}
      </div>
    </form>
  );
}
