import { TableIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { Loading } from '../components/Loading';
import { ButtonLink } from '../components/links';
import { reef } from '../lib/api';
import type { ProjectSummary } from '../lib/projects';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/projects')({ component: RepoProjects });

function RepoProjects() {
  const repo = useRepo();
  const q = useQuery({ queryKey: ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase(), 'projects'], queryFn: () => reef<ProjectSummary[]>(`/repos/${repo.full_name}/projects`) });
  if (q.isPending) return <Loading />;
  return (
    <div>
      <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 16 }}>
        <ButtonLink to="/$owner/projects" params={{ owner: repo.owner.login }} search={{}} variant="primary">
          {repo.owner.login}'s projects
        </ButtonLink>
      </div>
      <div className="issue-list">
        {q.data?.map((p) => (
          <div key={p.id} className="issue-row">
            <span className="state">
              <TableIcon className="muted" />
            </span>
            <div className="main">
              <div className="title-line">
                <Link to="/$owner/projects/$number" params={{ owner: p.owner.login, number: String(p.number) }} search={{}} className="title">
                  {p.title}
                </Link>
              </div>
              <div className="meta">#{p.number} · {p.short_description ?? ''}</div>
            </div>
          </div>
        ))}
        {q.data?.length === 0 ? (
          <Blankslate>
            <Blankslate.Visual>
              <TableIcon size="medium" />
            </Blankslate.Visual>
            <Blankslate.Heading>No linked projects</Blankslate.Heading>
            <Blankslate.Description>Link a project to this repository from the project's settings to see it here.</Blankslate.Description>
          </Blankslate>
        ) : null}
      </div>
    </div>
  );
}
