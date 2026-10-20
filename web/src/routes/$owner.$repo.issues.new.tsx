import { Avatar, Button, FormControl, TextInput } from '@primer/react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { LabelBadge, MarkdownEditor } from '../components/IssueBits';
import { ErrorFlash, Loading } from '../components/Loading';
import { api } from '../lib/api';
import { issueTemplatesQuery, labelsQuery, type Issue, type IssueTemplate } from '../lib/issues';
import { useSession } from '../lib/session';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/issues/new')({
  validateSearch: (s: Record<string, unknown>): { template?: string; title?: string; body?: string } => ({
    ...(typeof s.template === 'string' ? { template: s.template } : {}),
    ...(typeof s.title === 'string' ? { title: s.title } : {}),
    ...(typeof s.body === 'string' ? { body: s.body } : {}),
  }),
  component: NewIssue,
});

function NewIssue() {
  const repo = useRepo();
  const { user } = useSession();
  const navigate = useNavigate();
  const { template, title: titleParam, body: bodyParam } = Route.useSearch();
  const templates = useQuery(issueTemplatesQuery(repo.full_name, repo.default_branch));
  if (!user) {
    navigate({ to: '/login', search: { return_to: `/${repo.full_name}/issues/new` } });
    return null;
  }
  if (templates.isPending) return <Loading />;
  const list = templates.data ?? [];
  const chosen = template && template !== 'blank' ? list.find((t) => t.file === template) : undefined;
  if (list.length > 0 && !template) return <TemplateChooser repo={repo} templates={list} />;
  return <IssueForm repo={repo} template={chosen} initialTitle={titleParam} initialBody={bodyParam} />;
}

function TemplateChooser({ repo, templates }: { repo: ReturnType<typeof useRepo>; templates: IssueTemplate[] }) {
  const p = { owner: repo.owner.login, repo: repo.name };
  return (
    <div className="template-chooser">
      <div className="template-chooser-head">
        <h2>Create new issue</h2>
        <Link to="/$owner/$repo/issues/new" params={p} search={{ template: 'blank' }}>
          Open a blank issue
        </Link>
      </div>
      <div className="template-list">
        {templates.map((t) => (
          <div key={t.file} className="template-row">
            <div className="body">
              <strong>{t.name}</strong>
              {t.about ? <div className="muted small">{t.about}</div> : null}
            </div>
            <Link to="/$owner/$repo/issues/new" params={p} search={{ template: t.file }} className="btn-link">
              Get started
            </Link>
          </div>
        ))}
      </div>
      <p className="muted small" style={{ marginTop: 12 }}>
        Don't see your issue here?{' '}
        <Link to="/$owner/$repo/issues/new" params={p} search={{ template: 'blank' }}>
          Open a blank issue.
        </Link>
      </p>
    </div>
  );
}

function IssueForm({ repo, template, initialTitle, initialBody }: { repo: ReturnType<typeof useRepo>; template?: IssueTemplate; initialTitle?: string; initialBody?: string }) {
  const { user } = useSession();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const labels = useQuery(labelsQuery(repo.full_name));
  const [title, setTitle] = useState(initialTitle ?? template?.title ?? '');
  const [body, setBody] = useState(initialBody ?? template?.body ?? '');
  useEffect(() => {
    setTitle(initialTitle ?? template?.title ?? '');
    setBody(initialBody ?? template?.body ?? '');
  }, [template?.file, initialTitle, initialBody]); // eslint-disable-line react-hooks/exhaustive-deps
  const create = useMutation({
    mutationFn: () => api<Issue>(`/repos/${repo.full_name}/issues`, { method: 'POST', json: { title, body, labels: template?.labels ?? [], assignees: template?.assignees ?? [] } }),
    onSuccess: (i) => {
      qc.invalidateQueries({ queryKey: ['issues', repo.full_name.toLowerCase()] });
      navigate({ to: '/$owner/$repo/issues/$number', params: { owner: repo.owner.login, repo: repo.name, number: String(i.number) } });
    },
  });
  const templateLabels = (labels.data ?? []).filter((l) => template?.labels.some((n) => n.toLowerCase() === l.name.toLowerCase()));
  return (
    <div style={{ display: 'flex', gap: 16, maxWidth: 900 }}>
      {user ? <Avatar src={user.avatar_url} size={40} alt="" /> : null}
      <form
        className="form-stack"
        style={{ flex: 1 }}
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        <h2 style={{ margin: 0, fontSize: 24, fontWeight: 400 }}>Create new issue</h2>
        {template ? (
          <p className="muted small" style={{ margin: 0 }}>
            Using the <strong>{template.name}</strong> template
            {templateLabels.length ? (
              <>
                {' '}
                · labels:{' '}
                {templateLabels.map((l) => (
                  <LabelBadge key={l.id} label={l} small />
                ))}
              </>
            ) : null}
          </p>
        ) : null}
        {create.isError ? <ErrorFlash error={create.error} /> : null}
        <FormControl required>
          <FormControl.Label>Add a title</FormControl.Label>
          <TextInput block value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" autoFocus />
        </FormControl>
        <FormControl>
          <FormControl.Label>Add a description</FormControl.Label>
          <MarkdownEditor value={body} onChange={setBody} repo={repo} minHeight={200} />
        </FormControl>
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
          <Button onClick={() => history.back()}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={!title.trim()} loading={create.isPending}>
            Create
          </Button>
        </div>
      </form>
    </div>
  );
}
