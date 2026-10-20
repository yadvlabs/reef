import { ActionList, ActionMenu, Button, FormControl, TextInput } from '@primer/react';
import { GitBranchIcon, ArrowLeftIcon, GitCommitIcon } from '@primer/octicons-react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { FilesChanged } from '../components/Diff';
import { MarkdownEditor } from '../components/IssueBits';
import { ErrorFlash, Loading } from '../components/Loading';
import { api } from '../lib/api';
import { compareQuery, type Pull } from '../lib/pulls';
import { pullTemplateQuery } from '../lib/issues';
import { branchesQuery } from '../lib/repo';
import { useSession } from '../lib/session';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/compare/$')({ component: Compare });

function Compare() {
  const repo = useRepo();
  const { _splat } = Route.useParams();
  const navigate = useNavigate();
  const { user } = useSession();
  const branches = useQuery(branchesQuery(repo.owner.login, repo.name));
  const p = { owner: repo.owner.login, repo: repo.name };
  const spec = (_splat ?? '').replace(/^\/+/, '');
  const [baseIn, headIn] = spec.includes('...') ? spec.split('...') : [repo.default_branch, spec];
  const base = baseIn || repo.default_branch;
  const head = headIn || '';
  const cmp = useQuery(compareQuery(repo.full_name, base, head));
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const tmpl = useQuery(pullTemplateQuery(repo.full_name, repo.default_branch));
  useEffect(() => {
    if (tmpl.data && !body) setBody(tmpl.data);
  }, [tmpl.data]); // eslint-disable-line react-hooks/exhaustive-deps
  const create = useMutation({
    mutationFn: (draft: boolean) => api<Pull>(`/repos/${repo.full_name}/pulls`, { method: 'POST', json: { title: title || cmp.data?.commits[0]?.commit.message.split('\n')[0], body, head, base, draft } }),
    onSuccess: (pr) => navigate({ to: '/$owner/$repo/pull/$number', params: { ...p, number: String(pr.number) }, search: {} }),
  });
  const pick = (which: 'base' | 'head') => (name: string) => navigate({ to: '/$owner/$repo/compare/$', params: { ...p, _splat: which === 'base' ? `${name}...${head}` : `${base}...${name}` } });
  const Picker = ({ label, value, onPick }: { label: string; value: string; onPick: (n: string) => void }) => (
    <ActionMenu>
      <ActionMenu.Button leadingVisual={GitBranchIcon}>
        {label}: <strong>{value || 'choose a branch'}</strong>
      </ActionMenu.Button>
      <ActionMenu.Overlay width="medium">
        <ActionList selectionVariant="single">
          {branches.data?.map((b) => (
            <ActionList.Item key={b.name} selected={b.name === value} onSelect={() => onPick(b.name)}>
              {b.name}
            </ActionList.Item>
          ))}
        </ActionList>
      </ActionMenu.Overlay>
    </ActionMenu>
  );
  return (
    <div>
      <h2 style={{ fontSize: 24, fontWeight: 400, margin: '0 0 4px' }}>Comparing changes</h2>
      <p className="muted">Choose two branches to see what's changed or to start a new pull request.</p>
      <div className="compare-picker">
        <Picker label="base" value={base} onPick={pick('base')} />
        <ArrowLeftIcon className="muted" />
        <Picker label="compare" value={head} onPick={pick('head')} />
      </div>
      {!head ? null : cmp.isPending ? (
        <Loading />
      ) : cmp.isError ? (
        <ErrorFlash error={cmp.error} />
      ) : cmp.data.total_commits === 0 ? (
        <div style={{ border: '1px solid var(--borderColor-default)', borderRadius: 6, padding: 24, textAlign: 'center' }}>
          <strong>There isn't anything to compare.</strong>
          <p className="muted" style={{ margin: '4px 0 0' }}>
            <code>{base}</code> is up to date with all commits from <code>{head}</code>.
          </p>
        </div>
      ) : (
        <>
          {user ? (
            <form
              className="form-stack"
              style={{ border: '1px solid var(--borderColor-default)', borderRadius: 6, padding: 16, marginBottom: 24 }}
              onSubmit={(e) => {
                e.preventDefault();
                create.mutate(false);
              }}
            >
              {create.isError ? <ErrorFlash error={create.error} /> : null}
              <FormControl>
                <FormControl.Label>Add a title</FormControl.Label>
                <TextInput block value={title} onChange={(e) => setTitle(e.target.value)} placeholder={cmp.data.commits[0]?.commit.message.split('\n')[0] ?? 'Title'} />
              </FormControl>
              <FormControl>
                <FormControl.Label>Add a description</FormControl.Label>
                <MarkdownEditor value={body} onChange={setBody} repo={repo} minHeight={140} />
              </FormControl>
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
                <Button type="button" onClick={() => create.mutate(true)}>
                  Create draft
                </Button>
                <Button type="submit" variant="primary" loading={create.isPending}>
                  Create pull request
                </Button>
              </div>
            </form>
          ) : (
            <p className="muted">
              <Link to="/login" search={{ return_to: location.pathname }}>Sign in</Link> to create a pull request.
            </p>
          )}
          <div className="muted" style={{ marginBottom: 8, display: 'flex', gap: 12 }}>
            <span>
              <GitCommitIcon /> <strong>{cmp.data.total_commits}</strong> {cmp.data.total_commits === 1 ? 'commit' : 'commits'}
            </span>
            <span>
              <strong>{cmp.data.files.length}</strong> files changed
            </span>
            {cmp.data.behind_by > 0 ? <span>{cmp.data.behind_by} commits behind {base}</span> : null}
          </div>
          <div className="commit-list" style={{ marginBottom: 24 }}>
            <div className="group">
              {cmp.data.commits.map((c) => (
                <div key={c.sha} className="commit">
                  <div className="body">
                    <Link to="/$owner/$repo/commit/$sha" params={{ ...p, sha: c.sha }} className="title">
                      {c.commit.message.split('\n')[0]}
                    </Link>
                    <div className="who">
                      <strong>{c.author?.login ?? c.commit.author.name}</strong> committed
                    </div>
                  </div>
                  <code className="sha">{c.sha.slice(0, 7)}</code>
                </div>
              ))}
            </div>
          </div>
          <FilesChanged files={cmp.data.files} />
        </>
      )}
    </div>
  );
}
