import { Button, FormControl, TextInput, Textarea, Checkbox } from '@primer/react';
import { TableIcon, ProjectIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { AppHeader } from '../components/AppHeader';
import { accountQuery, OwnerTabs, useOwnerRole } from '../components/OwnerTabs';
import { ErrorFlash, Loading } from '../components/Loading';
import { reef } from '../lib/api';
import { projectsQuery, type ProjectSummary } from '../lib/projects';
import { useSession } from '../lib/session';

export const Route = createFileRoute('/$owner/projects/')({
  validateSearch: (s: Record<string, unknown>): { state?: string; new?: boolean } => ({ ...(typeof s.state === 'string' ? { state: s.state } : {}), ...(s.new === true ? { new: true } : {}) }),
  component: Projects,
});

function Projects() {
  const { owner } = Route.useParams();
  const { state = 'open', new: startNew } = Route.useSearch();
  const { user } = useSession();
  const account = useQuery(accountQuery(owner));
  const role = useOwnerRole(owner);
  const qc = useQueryClient();
  const navigate = useNavigate();
  const list = useQuery(projectsQuery(owner, state));
  const [creating, setCreating] = useState(!!startNew);
  const create = useMutation({
    mutationFn: (v: { title: string; short_description: string; public: boolean }) => reef<ProjectSummary>(`/projects/${owner}`, { method: 'POST', json: v }),
    onSuccess: (p) => {
      qc.invalidateQueries({ queryKey: ['projects', owner.toLowerCase()] });
      navigate({ to: '/$owner/projects/$number', params: { owner, number: String(p.number) }, search: {} });
    },
  });
  return (
    <>
      <AppHeader context={{ owner, section: { label: 'Projects', to: 'projects' } }} />
      {account.data ? <OwnerTabs account={account.data} current="projects" counts={{ projects: list.data?.length }} /> : null}
      <div className="page narrow">
        <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 16 }}>
          <h1 style={{ fontSize: 24, fontWeight: 400, margin: 0, flex: 1 }}>Projects</h1>
          {user && role.member ? (
            <Button variant="primary" onClick={() => setCreating(true)}>
              New project
            </Button>
          ) : null}
        </div>
        {create.isError ? <ErrorFlash error={create.error} /> : null}
        {creating ? (
          <form
            className="form-stack"
            style={{ border: '1px solid var(--borderColor-default)', borderRadius: 6, padding: 16, marginBottom: 16 }}
            onSubmit={(e) => {
              e.preventDefault();
              const f = new FormData(e.currentTarget);
              create.mutate({ title: String(f.get('title')), short_description: String(f.get('description') ?? ''), public: f.get('public') === 'on' });
            }}
          >
            <FormControl required>
              <FormControl.Label>Project name</FormControl.Label>
              <TextInput name="title" block autoFocus />
            </FormControl>
            <FormControl>
              <FormControl.Label>Short description</FormControl.Label>
              <Textarea name="description" block rows={2} />
            </FormControl>
            <FormControl>
              <Checkbox name="public" />
              <FormControl.Label>Public</FormControl.Label>
              <FormControl.Caption>Anyone who can see the owner can see this project. Otherwise it is visible to members only.</FormControl.Caption>
            </FormControl>
            <div style={{ display: 'flex', gap: 8 }}>
              <Button type="submit" variant="primary" loading={create.isPending}>
                Create project
              </Button>
              <Button type="button" onClick={() => setCreating(false)}>
                Cancel
              </Button>
            </div>
          </form>
        ) : null}
        <div className="issue-list">
          <div className="list-header">
            <Link to="/$owner/projects" params={{ owner }} search={{ state: 'open' }} style={{ fontWeight: state === 'open' ? 600 : 400, color: 'var(--fgColor-default)' }}>
              Open
            </Link>
            <Link to="/$owner/projects" params={{ owner }} search={{ state: 'closed' }} style={{ fontWeight: state === 'closed' ? 600 : 400, color: 'var(--fgColor-default)' }}>
              Closed
            </Link>
          </div>
          {list.isPending ? <Loading /> : null}
          {list.data?.map((p) => (
            <div key={p.id} className="issue-row">
              <span className="state">
                <TableIcon className="muted" />
              </span>
              <div className="main">
                <div className="title-line">
                  <Link to="/$owner/projects/$number" params={{ owner, number: String(p.number) }} search={{}} className="title">
                    {p.title}
                  </Link>
                  {!p.public ? <span className="small muted" style={{ border: '1px solid var(--borderColor-default)', borderRadius: 999, padding: '0 7px' }}>Private</span> : null}
                </div>
                <div className="meta">
                  #{p.number} · {p.items_count ?? 0} items · updated {new Date(p.updated_at).toLocaleDateString()}
                  {p.short_description ? ` · ${p.short_description}` : ''}
                </div>
              </div>
            </div>
          ))}
          {list.data?.length === 0 ? (
            <Blankslate>
              <Blankslate.Visual>
                <ProjectIcon size="medium" />
              </Blankslate.Visual>
              <Blankslate.Heading>No {state} projects</Blankslate.Heading>
              <Blankslate.Description>Projects are boards and tables for planning and tracking work across repositories.</Blankslate.Description>
            </Blankslate>
          ) : null}
        </div>
      </div>
    </>
  );
}
