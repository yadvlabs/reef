import { Button } from '@primer/react';
import { AlertIcon, LawIcon, PackageDependenciesIcon, ShieldIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { ErrorFlash, Loading } from '../components/Loading';
import { securityQuery } from '../lib/repo';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/security/')({ component: Overview });

function Overview() {
  const repo = useRepo();
  const q = useQuery(securityQuery(repo.owner.login, repo.name));
  const p = { owner: repo.owner.login, repo: repo.name };
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const d = q.data;
  const manifests = d.dependencies.length;
  return (
    <>
      <div className="security-head">
        <h2>Security overview</h2>
      </div>
      <section className="security-card">
        <div className="security-card-icon">
          <LawIcon size={24} />
        </div>
        <div className="security-card-body">
          <h3>Security policy</h3>
          {d.policy ? (
            <p>
              This repository has a security policy in <code>{d.policy.path}</code> that tells people how to report vulnerabilities.
            </p>
          ) : (
            <p>Define how users should report security vulnerabilities for this repository by adding a SECURITY.md file.</p>
          )}
        </div>
        <div className="security-card-action">
          {d.policy ? (
            <Link to="/$owner/$repo/security/policy" params={p}>
              <Button size="small">View security policy</Button>
            </Link>
          ) : repo.permissions?.push ? (
            <Link to="/$owner/$repo/new/$rev/$" params={{ ...p, rev: repo.default_branch, _splat: '' }}>
              <Button size="small">Suggest a security policy</Button>
            </Link>
          ) : null}
        </div>
      </section>
      <section className="security-card">
        <div className="security-card-icon">
          <AlertIcon size={24} />
        </div>
        <div className="security-card-body">
          <h3>Security advisories</h3>
          <p>View or disclose security advisories for this repository. No advisories have been published.</p>
        </div>
        <div className="security-card-action">
          <Link to="/$owner/$repo/security/advisories" params={p}>
            <Button size="small">View security advisories</Button>
          </Link>
        </div>
      </section>
      <section className="security-card">
        <div className="security-card-icon">
          <PackageDependenciesIcon size={24} />
        </div>
        <div className="security-card-body">
          <h3>Dependency graph</h3>
          <p>
            {d.total_dependencies === 0
              ? 'No dependency manifests were found on the default branch.'
              : `${d.total_dependencies} ${d.total_dependencies === 1 ? 'dependency' : 'dependencies'} declared across ${manifests} ${manifests === 1 ? 'manifest' : 'manifests'} on ${repo.default_branch}.`}
          </p>
        </div>
        <div className="security-card-action">
          <Link to="/$owner/$repo/security/dependencies" params={p}>
            <Button size="small">View dependencies</Button>
          </Link>
        </div>
      </section>
      <p className="muted small" style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 16 }}>
        <ShieldIcon /> Vulnerability alerts and code scanning are not available on this instance yet.
      </p>
    </>
  );
}
