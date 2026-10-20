import { Blankslate } from '@primer/react/experimental';
import { RepoIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { TreeView } from '../components/RepoCode';
import { RepoSidebar } from '../components/RepoSidebar';
import { type Repo } from '../lib/api';
import { branchesQuery } from '../lib/repo';
import { useSession } from '../lib/session';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/')({ component: RepoHome });

function RepoHome() {
  const repo = useRepo();
  const branches = useQuery(branchesQuery(repo.owner.login, repo.name));
  const empty = branches.data && branches.data.length === 0;
  return (
    <div className="code-layout">
      <div>{empty ? <EmptyRepo repo={repo} /> : branches.data ? <TreeView repo={repo} rev={repo.default_branch} path="" /> : null}</div>
      <RepoSidebar repo={repo} empty={!!empty} />
    </div>
  );
}

function EmptyRepo({ repo }: { repo: Repo }) {
  const { instance } = useSession();
  const url = `${instance?.site_url ?? location.origin}/${repo.full_name}.git`;
  const canPush = repo.permissions?.push;
  return (
    <div>
      <Blankslate border>
        <Blankslate.Visual>
          <RepoIcon size="medium" />
        </Blankslate.Visual>
        <Blankslate.Heading>This repository is empty</Blankslate.Heading>
        <Blankslate.Description>{canPush ? 'Push some code, or create a file in the browser to get started.' : 'Nothing has been pushed yet.'}</Blankslate.Description>
        {canPush ? <Blankslate.PrimaryAction href={`/${repo.full_name}/new/${repo.default_branch}/`}>Create a new file</Blankslate.PrimaryAction> : null}
      </Blankslate>
      {canPush ? (
        <div style={{ marginTop: 24 }}>
          <h3 style={{ fontSize: 16 }}>…or create a new repository on the command line</h3>
          <pre className="markdown-body" style={{ padding: 16, background: 'var(--bgColor-muted)', borderRadius: 6, fontSize: 12 }}>
            {`echo "# ${repo.name}" >> README.md
git init
git add README.md
git commit -m "first commit"
git branch -M ${repo.default_branch}
git remote add origin ${url}
git push -u origin ${repo.default_branch}`}
          </pre>
          <h3 style={{ fontSize: 16 }}>…or push an existing repository from the command line</h3>
          <pre className="markdown-body" style={{ padding: 16, background: 'var(--bgColor-muted)', borderRadius: 6, fontSize: 12 }}>
            {`git remote add origin ${url}
git branch -M ${repo.default_branch}
git push -u origin ${repo.default_branch}`}
          </pre>
          <p className="muted small">
            Authenticate with your username and a <Link to="/settings/tokens">personal access token</Link>, or use SSH with a registered key.
          </p>
        </div>
      ) : null}
    </div>
  );
}

