import { Button, Checkbox, FormControl, TextInput, Label } from '@primer/react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute } from '@tanstack/react-router';
import { useState } from 'react';
import { ErrorFlash, NotFound } from '../components/Loading';
import { SettingsNav } from '../components/SettingsNav';
import { api } from '../lib/api';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/settings/branches')({ component: Branches });

interface Rule {
  pattern: string;
  enforce_admins: { enabled: boolean };
  required_pull_request_reviews: { required_approving_review_count: number; dismiss_stale_reviews: boolean } | null;
  required_status_checks: { strict: boolean; contexts: string[] };
  required_linear_history: { enabled: boolean };
  allow_force_pushes: { enabled: boolean };
  allow_deletions: { enabled: boolean };
  lock_branch: { enabled: boolean };
}

function Branches() {
  const repo = useRepo();
  const qc = useQueryClient();
  const key = ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase(), 'protection-rules'];
  const rules = useQuery({ queryKey: key, queryFn: () => api<Rule[]>(`/repos/${repo.full_name}/protection-rules`) });
  const [editing, setEditing] = useState<Rule | 'new' | null>(null);
  const [error, setError] = useState<unknown>(null);
  if (!repo.permissions?.admin) return <NotFound />;

  async function save(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setError(null);
    const f = new FormData(e.currentTarget);
    const pattern = String(f.get('pattern') ?? '').trim();
    const requirePr = f.get('require_pr') === 'on';
    const contexts = String(f.get('contexts') ?? '').split(',').map((s) => s.trim()).filter(Boolean);
    const body = {
      required_status_checks: contexts.length ? { strict: f.get('strict') === 'on', contexts } : null,
      enforce_admins: f.get('enforce_admins') === 'on',
      required_pull_request_reviews: requirePr ? { required_approving_review_count: Number(f.get('approvals') ?? 0), dismiss_stale_reviews: f.get('dismiss_stale') === 'on' } : null,
      required_linear_history: f.get('linear') === 'on',
      allow_force_pushes: f.get('force') === 'on',
      allow_deletions: f.get('deletions') === 'on',
      lock_branch: f.get('lock') === 'on',
    };
    try {
      await api(`/repos/${repo.full_name}/protection-rules/${pattern.split('/').map(encodeURIComponent).join('/')}`, { method: 'PUT', json: body });
      qc.invalidateQueries({ queryKey: key });
      qc.invalidateQueries({ queryKey: ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase(), 'branches'] });
      setEditing(null);
    } catch (err) {
      setError(err);
    }
  }
  async function remove(r: Rule) {
    if (!confirm(`Delete the rule for "${r.pattern}"?`)) return;
    await api(`/repos/${repo.full_name}/protection-rules/${r.pattern.split('/').map(encodeURIComponent).join('/')}`, { method: 'DELETE' });
    qc.invalidateQueries({ queryKey: key });
  }
  const cur = editing === 'new' ? null : editing;
  return (
    <div className="settings-layout">
      <SettingsNav repo={repo} current="branches" />
      <div className="form-stack">
        <h2 className="section-title" style={{ display: 'flex', alignItems: 'center' }}>
          Branch protection rules
          <span style={{ flex: 1 }} />
          <Button size="small" onClick={() => setEditing('new')}>
            Add rule
          </Button>
        </h2>
        {error ? <ErrorFlash error={error} /> : null}
        {editing ? (
          <form className="form-stack" onSubmit={save} style={{ border: '1px solid var(--borderColor-default)', borderRadius: 6, padding: 16 }}>
            <FormControl required>
              <FormControl.Label>Branch name pattern</FormControl.Label>
              <TextInput name="pattern" defaultValue={cur?.pattern ?? ''} placeholder="main or release/*" monospace readOnly={!!cur} />
              <FormControl.Caption>Matches branch names. `*` matches within one path segment, `**` across segments.</FormControl.Caption>
            </FormControl>
            <FormControl>
              <Checkbox name="require_pr" defaultChecked={!!cur?.required_pull_request_reviews} />
              <FormControl.Label>Require a pull request before merging</FormControl.Label>
              <FormControl.Caption>Direct pushes to matching branches are rejected.</FormControl.Caption>
            </FormControl>
            <div style={{ display: 'flex', gap: 16, marginLeft: 24 }}>
              <FormControl>
                <FormControl.Label>Required approvals</FormControl.Label>
                <TextInput name="approvals" type="number" defaultValue={String(cur?.required_pull_request_reviews?.required_approving_review_count ?? 1)} min={0} max={6} style={{ width: 80 }} />
              </FormControl>
              <FormControl>
                <Checkbox name="dismiss_stale" defaultChecked={cur?.required_pull_request_reviews?.dismiss_stale_reviews ?? false} />
                <FormControl.Label>Dismiss stale approvals when new commits are pushed</FormControl.Label>
              </FormControl>
            </div>
            <FormControl>
              <FormControl.Label>Required status checks</FormControl.Label>
              <TextInput name="contexts" defaultValue={cur?.required_status_checks?.contexts.join(', ') ?? ''} placeholder="build, test" block />
              <FormControl.Caption>Comma separated check names that must pass before merging.</FormControl.Caption>
            </FormControl>
            <FormControl>
              <Checkbox name="strict" defaultChecked={cur?.required_status_checks?.strict ?? false} />
              <FormControl.Label>Require branches to be up to date before merging</FormControl.Label>
            </FormControl>
            <FormControl>
              <Checkbox name="linear" defaultChecked={cur?.required_linear_history.enabled ?? false} />
              <FormControl.Label>Require linear history</FormControl.Label>
            </FormControl>
            <FormControl>
              <Checkbox name="enforce_admins" defaultChecked={cur?.enforce_admins.enabled ?? false} />
              <FormControl.Label>Do not allow bypassing the above settings</FormControl.Label>
              <FormControl.Caption>Applies the rules to administrators too.</FormControl.Caption>
            </FormControl>
            <FormControl>
              <Checkbox name="force" defaultChecked={cur?.allow_force_pushes.enabled ?? false} />
              <FormControl.Label>Allow force pushes</FormControl.Label>
            </FormControl>
            <FormControl>
              <Checkbox name="deletions" defaultChecked={cur?.allow_deletions.enabled ?? false} />
              <FormControl.Label>Allow deletions</FormControl.Label>
            </FormControl>
            <FormControl>
              <Checkbox name="lock" defaultChecked={cur?.lock_branch.enabled ?? false} />
              <FormControl.Label>Lock branch</FormControl.Label>
              <FormControl.Caption>The branch becomes read-only.</FormControl.Caption>
            </FormControl>
            <div style={{ display: 'flex', gap: 8 }}>
              <Button type="submit" variant="primary">
                {cur ? 'Save changes' : 'Create'}
              </Button>
              <Button onClick={() => setEditing(null)}>Cancel</Button>
            </div>
          </form>
        ) : null}
        <div className="key-list">
          {rules.data?.map((r) => (
            <div key={r.pattern} className="item" style={{ alignItems: 'center' }}>
              <div className="body">
                <code>{r.pattern}</code>
                <div style={{ marginTop: 4, display: 'flex', gap: 4, flexWrap: 'wrap' }}>
                  {r.required_pull_request_reviews ? <Label>Pull request required · {r.required_pull_request_reviews.required_approving_review_count} approvals</Label> : null}
                  {r.required_status_checks.contexts.length ? <Label>Checks: {r.required_status_checks.contexts.join(', ')}</Label> : null}
                  {r.required_linear_history.enabled ? <Label>Linear history</Label> : null}
                  {r.enforce_admins.enabled ? <Label variant="attention">Includes administrators</Label> : null}
                  {r.allow_force_pushes.enabled ? <Label variant="secondary">Force pushes allowed</Label> : <Label variant="secondary">No force pushes</Label>}
                  {r.allow_deletions.enabled ? <Label variant="secondary">Deletions allowed</Label> : <Label variant="secondary">No deletions</Label>}
                  {r.lock_branch.enabled ? <Label variant="danger">Locked</Label> : null}
                </div>
              </div>
              <Button size="small" onClick={() => setEditing(r)}>
                Edit
              </Button>
              <Button size="small" variant="danger" onClick={() => remove(r)}>
                Delete
              </Button>
            </div>
          ))}
          {rules.data?.length === 0 && !editing ? <p className="muted">No branch protection rules yet.</p> : null}
        </div>
      </div>
    </div>
  );
}
