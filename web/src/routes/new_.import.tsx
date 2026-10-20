import { Button, FormControl, Radio, RadioGroup, Select, TextInput } from '@primer/react';
import { RepoIcon, LockIcon, CheckCircleFillIcon, XCircleFillIcon } from '@primer/octicons-react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { AppHeader } from '../components/AppHeader';
import { ErrorFlash, Loading } from '../components/Loading';
import { api, type Repo } from '../lib/api';
import { importQuery, startImport } from '../lib/mirror';
import { useSession } from '../lib/session';

export const Route = createFileRoute('/new_/import')({
  validateSearch: (s: Record<string, unknown>): { owner?: string; repo?: string } => ({
    ...(typeof s.owner === 'string' ? { owner: s.owner } : {}),
    ...(typeof s.repo === 'string' ? { repo: s.repo } : {}),
  }),
  component: ImportRepo,
});

function ImportRepo() {
  const { user, orgs, loading } = useSession();
  const search = Route.useSearch();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);
  const [visibility, setVisibility] = useState('public');
  const [started, setStarted] = useState<string | null>(search.repo ?? null);
  if (loading) return <Loading />;
  if (!user) {
    navigate({ to: '/login', search: { return_to: '/new/import' } });
    return null;
  }
  if (started) return <Progress full={started} />;

  async function submit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    const f = new FormData(e.currentTarget);
    const owner = String(f.get('owner'));
    const name = String(f.get('name') ?? '').trim();
    const body = { name, private: visibility === 'private', auto_init: false };
    try {
      const repo = owner === user!.login ? await api<Repo>('/user/repos', { method: 'POST', json: body }) : await api<Repo>(`/orgs/${owner}/repos`, { method: 'POST', json: body });
      await startImport(repo.full_name, { vcs_url: String(f.get('vcs_url') ?? '').trim(), vcs_username: String(f.get('vcs_username') ?? '') || undefined, vcs_password: String(f.get('vcs_password') ?? '') || undefined });
      qc.invalidateQueries({ queryKey: ['user', 'repos'] });
      setStarted(repo.full_name);
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
        <h1 className="section-title">Import your project to Reef</h1>
        <p className="muted">Commits, branches and tags are copied from the source repository. Issues, pull requests and wiki pages are not imported. Git LFS objects are skipped.</p>
        {error ? <div style={{ marginBottom: 16 }}><ErrorFlash error={error} /></div> : null}
        <form className="form-stack" onSubmit={submit}>
          <FormControl required>
            <FormControl.Label>Your old repository's clone URL</FormControl.Label>
            <TextInput name="vcs_url" block monospace placeholder="https://github.com/owner/repo.git" autoFocus />
            <FormControl.Caption>Git over HTTPS, SSH or git:// works. For private sources enter credentials below; they are used for the import only.</FormControl.Caption>
          </FormControl>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8 }}>
            <FormControl>
              <FormControl.Label>Username</FormControl.Label>
              <TextInput name="vcs_username" block autoComplete="off" />
            </FormControl>
            <FormControl>
              <FormControl.Label>Password or token</FormControl.Label>
              <TextInput name="vcs_password" type="password" block autoComplete="new-password" />
            </FormControl>
          </div>
          <div style={{ display: 'flex', gap: 8, alignItems: 'flex-end' }}>
            <FormControl>
              <FormControl.Label>Owner</FormControl.Label>
              <Select name="owner" defaultValue={search.owner ?? user.login}>
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
              <TextInput name="name" />
            </FormControl>
          </div>
          <RadioGroup name="visibility" onChange={(v) => setVisibility(v ?? 'public')}>
            <RadioGroup.Label>Privacy</RadioGroup.Label>
            <FormControl>
              <Radio value="public" defaultChecked />
              <FormControl.Label>
                <RepoIcon /> Public
              </FormControl.Label>
            </FormControl>
            <FormControl>
              <Radio value="private" />
              <FormControl.Label>
                <LockIcon /> Private
              </FormControl.Label>
            </FormControl>
          </RadioGroup>
          <div style={{ display: 'flex', gap: 8 }}>
            <Button type="submit" variant="primary" loading={busy}>
              Begin import
            </Button>
            <Button as="a" href="/new" variant="invisible">
              Cancel
            </Button>
          </div>
        </form>
      </div>
    </>
  );
}

function Progress({ full }: { full: string }) {
  const q = useQuery({ ...importQuery(full), refetchInterval: (query) => (query.state.data?.status === 'importing' ? 2000 : false) });
  const [owner, repo] = full.split('/');
  const st = q.data;
  return (
    <>
      <AppHeader context={{ owner, repo }} />
      <div className="page form">
        <h1 className="section-title">Importing {full}</h1>
        <div className="import-progress">
          {!st || st.status === 'importing' ? (
            <>
              <div className="spinner" />
              <strong>{st?.status_text ?? 'Starting import'}</strong>
              <p className="muted small">Fetching from {st?.vcs_url ?? 'the source'}. This page updates automatically.</p>
            </>
          ) : st.status === 'complete' ? (
            <>
              <CheckCircleFillIcon size={32} fill="var(--fgColor-success)" />
              <p>
                <strong>Import complete</strong> · {st.commit_count ?? 0} {st.commit_count === 1 ? 'commit' : 'commits'}
              </p>
              <Link to="/$owner/$repo" params={{ owner, repo }}>
                <Button variant="primary">Open {full}</Button>
              </Link>
            </>
          ) : (
            <>
              <XCircleFillIcon size={32} fill="var(--fgColor-danger)" />
              <p>
                <strong>Import {st.status === 'cancelled' ? 'cancelled' : 'failed'}</strong>
              </p>
              {st.message ? <p className="mirror-error">{st.message}</p> : null}
              <p className="small">
                <Link to="/$owner/$repo" params={{ owner, repo }}>Open the empty repository</Link> or retry from its settings.
              </p>
            </>
          )}
        </div>
      </div>
    </>
  );
}
