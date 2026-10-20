// The "Projects" section of an issue or pull request sidebar: which projects it is in, its
// Status there, and adding it to another project.
import { ActionList, ActionMenu } from '@primer/react';
import { TableIcon, XIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { reef, type Repo } from '../lib/api';
import type { ProjectField, ProjectSummary } from '../lib/projects';
import { OptionPill } from './ProjectBoard';

interface Membership {
  project: ProjectSummary;
  item_id: number;
  fields: ProjectField[];
  values: Record<string, string | null>;
}

export function IssueProjects({ repo, number, disabled }: { repo: Repo; number: number; disabled: boolean }) {
  const qc = useQueryClient();
  const key = ['issue-projects', repo.full_name.toLowerCase(), number];
  const q = useQuery({ queryKey: key, queryFn: () => reef<{ projects: Membership[]; available: ProjectSummary[] }>(`/repos/${repo.full_name}/issues/${number}/projects`) });
  const invalidate = () => {
    qc.invalidateQueries({ queryKey: key });
    qc.invalidateQueries({ queryKey: ['project'] });
  };
  const add = useMutation({ mutationFn: (p: ProjectSummary) => reef(`/projects/${p.owner.login}/${p.number}/items`, { method: 'POST', json: { issue: `${repo.full_name}#${number}` } }), onSuccess: invalidate });
  const remove = useMutation({ mutationFn: (m: Membership) => reef(`/projects/${m.project.owner.login}/${m.project.number}/items/${m.item_id}`, { method: 'DELETE' }), onSuccess: invalidate });
  const setValue = useMutation({ mutationFn: ({ m, field, value }: { m: Membership; field: number; value: string | null }) => reef(`/projects/${m.project.owner.login}/${m.project.number}/items/${m.item_id}`, { method: 'PATCH', json: { values: { [String(field)]: value } } }), onSuccess: invalidate });
  const available = q.data?.available ?? [];
  return (
    <section>
      <ActionMenu>
        <ActionMenu.Anchor>
          <button className="sidebar-head" disabled={disabled || available.length === 0}>
            Projects
          </button>
        </ActionMenu.Anchor>
        <ActionMenu.Overlay width="medium">
          <ActionList>
            <ActionList.Group>
              <ActionList.GroupHeading>Add to project</ActionList.GroupHeading>
              {available.map((p) => (
                <ActionList.Item key={p.id} onSelect={() => add.mutate(p)}>
                  <ActionList.LeadingVisual>
                    <TableIcon />
                  </ActionList.LeadingVisual>
                  {p.title}
                  <ActionList.Description>{p.owner.login}</ActionList.Description>
                </ActionList.Item>
              ))}
            </ActionList.Group>
          </ActionList>
        </ActionMenu.Overlay>
      </ActionMenu>
      <div className="sidebar-body">
        {q.data?.projects.length ? (
          q.data.projects.map((m) => {
            const status = m.fields.find((f) => f.data_type === 'single_select' && f.name.toLowerCase() === 'status') ?? m.fields.find((f) => f.data_type === 'single_select');
            return (
              <div key={m.item_id} className="issue-project">
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <TableIcon className="muted" />
                  <Link to="/$owner/projects/$number" params={{ owner: m.project.owner.login, number: String(m.project.number) }} search={{ item: m.item_id }} style={{ fontWeight: 600, color: 'var(--fgColor-default)', flex: 1 }}>
                    {m.project.title}
                  </Link>
                  {!disabled && m.project.permissions.write ? (
                    <button className="linkish muted" aria-label="Remove from project" onClick={() => confirm(`Remove from ${m.project.title}?`) && remove.mutate(m)}>
                      <XIcon size={14} />
                    </button>
                  ) : null}
                </div>
                {status ? (
                  <ActionMenu>
                    <ActionMenu.Anchor>
                      <button className="cell-button compact" disabled={disabled || !m.project.permissions.write} style={{ marginLeft: 22 }}>
                        <OptionPill field={status} value={m.values[String(status.id)]} />
                      </button>
                    </ActionMenu.Anchor>
                    <ActionMenu.Overlay>
                      <ActionList selectionVariant="single">
                        {status.options.map((o) => (
                          <ActionList.Item key={o.id} selected={m.values[String(status.id)] === o.id} onSelect={() => setValue.mutate({ m, field: status.id, value: o.id })}>
                            {o.name}
                            {o.description ? <ActionList.Description variant="block">{o.description}</ActionList.Description> : null}
                          </ActionList.Item>
                        ))}
                      </ActionList>
                    </ActionMenu.Overlay>
                  </ActionMenu>
                ) : null}
              </div>
            );
          })
        ) : (
          <span className="muted">None yet</span>
        )}
      </div>
    </section>
  );
}
