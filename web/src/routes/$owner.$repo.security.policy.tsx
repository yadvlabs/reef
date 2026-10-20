import { Button } from '@primer/react';
import { Blankslate } from '@primer/react/experimental';
import { LawIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { ErrorFlash, Loading } from '../components/Loading';
import { Markdown } from '../components/Markdown';
import { securityQuery } from '../lib/repo';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/security/policy')({ component: Policy });

function Policy() {
  const repo = useRepo();
  const q = useQuery(securityQuery(repo.owner.login, repo.name));
  const p = { owner: repo.owner.login, repo: repo.name };
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const policy = q.data.policy;
  return (
    <>
      <div className="security-head">
        <h2>Security policy</h2>
        <span style={{ flex: 1 }} />
        {policy && repo.permissions?.push ? (
          <Link to="/$owner/$repo/edit/$rev/$" params={{ ...p, rev: repo.default_branch, _splat: policy.path }}>
            <Button size="small">Edit policy</Button>
          </Link>
        ) : null}
      </div>
      {policy ? (
        <div className="blob">
          <div className="blob-header">
            <LawIcon />
            <Link to="/$owner/$repo/blob/$rev/$" params={{ ...p, rev: repo.default_branch, _splat: policy.path }}>
              {policy.path}
            </Link>
          </div>
          <Markdown html={policy.html} />
        </div>
      ) : (
        <Blankslate border spacious>
          <Blankslate.Visual>
            <LawIcon size="medium" />
          </Blankslate.Visual>
          <Blankslate.Heading>No security policy detected</Blankslate.Heading>
          <Blankslate.Description>
            This project has not set up a SECURITY.md file yet. A policy tells people how to report a vulnerability privately and what versions are supported.
          </Blankslate.Description>
          {repo.permissions?.push ? (
            <Blankslate.PrimaryAction href={`/${repo.full_name}/new/${repo.default_branch}/`}>Suggest a policy (name the file SECURITY.md)</Blankslate.PrimaryAction>
          ) : null}
        </Blankslate>
      )}
    </>
  );
}
