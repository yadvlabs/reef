import { Button, Checkbox, FormControl, Select, TextInput, Textarea } from '@primer/react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { SettingsNav } from '../components/SettingsNav';
import { api, type Repo } from '../lib/api';
import { branchesQuery, repoQuery } from '../lib/repo';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/settings/')({ component: Settings });

function Settings() {
  const repo = useRepo();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const branches = useQuery(branchesQuery(repo.owner.login, repo.name));
  const [error, setError] = useState<unknown>(null);
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);
  if (!repo.permissions?.admin) return <NotFound what="page" />;

  async function save(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    setSaved(false);
    const f = new FormData(e.currentTarget);
    const body: Record<string, unknown> = {
      name: String(f.get('name')),
      description: String(f.get('description') ?? ''),
      homepage: String(f.get('homepage') ?? ''),
      default_branch: String(f.get('default_branch')),
      has_issues: f.get('has_issues') === 'on',
      has_projects: f.get('has_projects') === 'on',
      has_wiki: f.get('has_wiki') === 'on',
      allow_merge_commit: f.get('allow_merge_commit') === 'on',
      allow_squash_merge: f.get('allow_squash_merge') === 'on',
      allow_rebase_merge: f.get('allow_rebase_merge') === 'on',
      delete_branch_on_merge: f.get('delete_branch_on_merge') === 'on',
    };
    try {
      const updated = await api<Repo>(`/repos/${repo.full_name}`, { method: 'PATCH', json: body });
      qc.invalidateQueries({ queryKey: ['repo'] });
      qc.invalidateQueries({ queryKey: ['user', 'repos'] });
      setSaved(true);
      if (updated.name !== repo.name) navigate({ to: '/$owner/$repo/settings', params: { owner: updated.owner.login, repo: updated.name } });
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  }

  async function setVisibility(priv: boolean) {
    if (!confirm(`Make this repository ${priv ? 'private' : 'public'}?`)) return;
    try {
      await api(`/repos/${repo.full_name}`, { method: 'PATCH', json: { private: priv } });
      qc.invalidateQueries({ queryKey: ['repo'] });
    } catch (err) {
      setError(err);
    }
  }

  async function archive(a: boolean) {
    if (!confirm(a ? 'Archive this repository? It becomes read-only.' : 'Unarchive this repository?')) return;
    try {
      await api(`/repos/${repo.full_name}`, { method: 'PATCH', json: { archived: a } });
      qc.invalidateQueries({ queryKey: ['repo'] });
    } catch (err) {
      setError(err);
    }
  }

  async function del() {
    const typed = prompt(`This will permanently delete ${repo.full_name}. Type the repository name to confirm.`);
    if (typed !== repo.full_name && typed !== repo.name) return;
    try {
      await api(`/repos/${repo.full_name}`, { method: 'DELETE' });
      qc.invalidateQueries({ queryKey: ['user', 'repos'] });
      qc.removeQueries({ queryKey: repoQuery(repo.owner.login, repo.name).queryKey });
      navigate({ to: '/' });
    } catch (err) {
      setError(err);
    }
  }

  return (
    <div className="settings-layout">
      <SettingsNav repo={repo} current="general" />
      <div className="form-stack">
        <h2 className="section-title">General</h2>
        {error ? <ErrorFlash error={error} /> : null}
        <form className="form-stack" onSubmit={save}>
          <FormControl required>
            <FormControl.Label>Repository name</FormControl.Label>
            <TextInput name="name" defaultValue={repo.name} />
          </FormControl>
          <FormControl>
            <FormControl.Label>Description</FormControl.Label>
            <Textarea name="description" defaultValue={repo.description ?? ''} block rows={2} />
          </FormControl>
          <FormControl>
            <FormControl.Label>Website</FormControl.Label>
            <TextInput name="homepage" defaultValue={repo.homepage ?? ''} block />
          </FormControl>
          <FormControl>
            <FormControl.Label>Default branch</FormControl.Label>
            {branches.isPending ? (
              <Loading />
            ) : (
              <Select name="default_branch" defaultValue={repo.default_branch}>
                {(branches.data?.length ? branches.data.map((b) => b.name) : [repo.default_branch]).map((n) => (
                  <Select.Option key={n} value={n}>
                    {n}
                  </Select.Option>
                ))}
              </Select>
            )}
          </FormControl>
          <h3 style={{ margin: '8px 0 0', fontSize: 16 }}>Features</h3>
          <FormControl>
            <Checkbox name="has_issues" defaultChecked={repo.has_issues} />
            <FormControl.Label>Issues</FormControl.Label>
          </FormControl>
          <FormControl>
            <Checkbox name="has_projects" defaultChecked={repo.has_projects} />
            <FormControl.Label>Projects</FormControl.Label>
          </FormControl>
          <FormControl>
            <Checkbox name="has_wiki" defaultChecked={repo.has_wiki} />
            <FormControl.Label>Wikis</FormControl.Label>
          </FormControl>
          <h3 style={{ margin: '8px 0 0', fontSize: 16 }}>Pull requests</h3>
          <FormControl>
            <Checkbox name="allow_merge_commit" defaultChecked={repo.allow_merge_commit ?? true} />
            <FormControl.Label>Allow merge commits</FormControl.Label>
          </FormControl>
          <FormControl>
            <Checkbox name="allow_squash_merge" defaultChecked={repo.allow_squash_merge ?? true} />
            <FormControl.Label>Allow squash merging</FormControl.Label>
          </FormControl>
          <FormControl>
            <Checkbox name="allow_rebase_merge" defaultChecked={repo.allow_rebase_merge ?? true} />
            <FormControl.Label>Allow rebase merging</FormControl.Label>
          </FormControl>
          <FormControl>
            <Checkbox name="delete_branch_on_merge" defaultChecked={repo.delete_branch_on_merge ?? false} />
            <FormControl.Label>Automatically delete head branches</FormControl.Label>
          </FormControl>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Button type="submit" variant="primary" loading={busy}>
              Save changes
            </Button>
            {saved ? <span className="muted">Saved.</span> : null}
          </div>
        </form>

        <h2 className="section-title" style={{ color: 'var(--fgColor-danger)' }}>Danger Zone</h2>
        <div className="danger-zone">
          <div className="row">
            <div className="text">
              <strong>Change repository visibility</strong>
              <div className="muted small">This repository is currently {repo.private ? 'private' : 'public'}.</div>
            </div>
            <Button variant="danger" onClick={() => setVisibility(!repo.private)}>
              Change visibility
            </Button>
          </div>
          <div className="row">
            <div className="text">
              <strong>{repo.archived ? 'Unarchive' : 'Archive'} this repository</strong>
              <div className="muted small">Mark this repository as {repo.archived ? 'active' : 'archived and read-only'}.</div>
            </div>
            <Button variant="danger" onClick={() => archive(!repo.archived)}>
              {repo.archived ? 'Unarchive' : 'Archive'} this repository
            </Button>
          </div>
          <div className="row">
            <div className="text">
              <strong>Delete this repository</strong>
              <div className="muted small">Once you delete a repository, there is no going back.</div>
            </div>
            <Button variant="danger" onClick={del}>
              Delete this repository
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

