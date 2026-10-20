import { Button, Checkbox, FormControl, Radio, RadioGroup, Select, TextInput, Textarea } from '@primer/react';
import { RepoIcon, LockIcon, RepoPushIcon } from '@primer/octicons-react';
import { useQueryClient } from '@tanstack/react-query';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading } from '../components/Loading';
import { api, type Repo } from '../lib/api';
import { useSession } from '../lib/session';

export const Route = createFileRoute('/new')({
  validateSearch: (s: Record<string, unknown>): { owner?: string } => (typeof s.owner === 'string' ? { owner: s.owner } : {}),
  component: NewRepo,
});

function NewRepo() {
  const { user, orgs, loading } = useSession();
  const { owner: initialOwner } = Route.useSearch();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [visibility, setVisibility] = useState('public');

  if (loading) return <Loading />;
  if (!user) {
    navigate({ to: '/login', search: { return_to: '/new' } });
    return null;
  }

  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const f = new FormData(e.currentTarget);
    const owner = String(f.get('owner'));
    const body = {
      name: String(f.get('name') ?? '').trim(),
      description: String(f.get('description') ?? ''),
      private: visibility === 'private',
      auto_init: f.get('auto_init') === 'on',
    };
    try {
      const repo = owner === user!.login ? await api<Repo>('/user/repos', { method: 'POST', json: body }) : await api<Repo>(`/orgs/${owner}/repos`, { method: 'POST', json: body });
      qc.invalidateQueries({ queryKey: ['user', 'repos'] });
      navigate({ to: '/$owner/$repo', params: { owner: repo.owner.login, repo: repo.name } });
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
        <h1 className="section-title">Create a new repository</h1>
        <p className="muted">
          A repository contains all project files, including the revision history. Already have a project elsewhere?{' '}
          <Link to="/new/import" search={{}}>
            Import a repository
          </Link>
          .
        </p>
        <Link to="/new/import" search={{ owner: initialOwner }} className="import-choice">
          <RepoPushIcon size={20} />
          <span>
            <strong>Import a repository</strong>
            <span className="muted small" style={{ display: 'block' }}>Copy commits, branches and tags from a Git URL on another host.</span>
          </span>
        </Link>
        {error ? <div style={{ marginBottom: 16 }}><ErrorFlash error={error} /></div> : null}
        <form className="form-stack" onSubmit={submit}>
          <div style={{ display: 'flex', gap: 8, alignItems: 'flex-end' }}>
            <FormControl>
              <FormControl.Label>Owner</FormControl.Label>
              <Select name="owner" defaultValue={initialOwner ?? user.login}>
                <Select.Option value={user.login}>{user.login}</Select.Option>
                {orgs.map((o) => (
                  <Select.Option key={o.id} value={o.login}>
                    {o.login}
                  </Select.Option>
                ))}
              </Select>
            </FormControl>
            <span style={{ fontSize: 20, paddingBottom: 6 }}>/</span>
            <FormControl required>
              <FormControl.Label>Repository name</FormControl.Label>
              <TextInput name="name" autoFocus />
            </FormControl>
          </div>
          <FormControl>
            <FormControl.Label>Description</FormControl.Label>
            <Textarea name="description" block rows={2} />
          </FormControl>
          <RadioGroup name="visibility" onChange={(v) => setVisibility(v ?? 'public')}>
            <RadioGroup.Label>Visibility</RadioGroup.Label>
            <FormControl>
              <Radio value="public" defaultChecked />
              <FormControl.Label>
                <RepoIcon /> Public
              </FormControl.Label>
              <FormControl.Caption>Anyone on this Reef can see this repository. You choose who can commit.</FormControl.Caption>
            </FormControl>
            <FormControl>
              <Radio value="private" />
              <FormControl.Label>
                <LockIcon /> Private
              </FormControl.Label>
              <FormControl.Caption>You choose who can see and commit to this repository.</FormControl.Caption>
            </FormControl>
          </RadioGroup>
          <FormControl>
            <Checkbox name="auto_init" />
            <FormControl.Label>Add a README file</FormControl.Label>
            <FormControl.Caption>This is where you can write a long description for your project.</FormControl.Caption>
          </FormControl>
          <div>
            <Button type="submit" variant="primary" loading={busy}>
              Create repository
            </Button>
          </div>
        </form>
      </div>
    </>
  );
}
