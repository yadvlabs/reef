import { Label } from '@primer/react';
import { Blankslate } from '@primer/react/experimental';
import { FileIcon, PackageDependenciesIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { ErrorFlash, Loading } from '../components/Loading';
import { securityQuery } from '../lib/repo';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/security/dependencies')({ component: Dependencies });

const ECOSYSTEMS: Record<string, string> = {
  cargo: 'Cargo',
  npm: 'npm',
  go: 'Go modules',
  pip: 'pip',
  maven: 'Maven',
  gradle: 'Gradle',
  composer: 'Composer',
  gem: 'RubyGems',
  nuget: 'NuGet',
};

function Dependencies() {
  const repo = useRepo();
  const q = useQuery(securityQuery(repo.owner.login, repo.name));
  const p = { owner: repo.owner.login, repo: repo.name };
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const list = q.data.dependencies;
  return (
    <>
      <div className="security-head">
        <h2>Dependencies</h2>
        <span className="muted small">
          {q.data.total_dependencies} {q.data.total_dependencies === 1 ? 'dependency' : 'dependencies'} on {repo.default_branch}
        </span>
      </div>
      {list.length === 0 ? (
        <Blankslate border spacious>
          <Blankslate.Visual>
            <PackageDependenciesIcon size="medium" />
          </Blankslate.Visual>
          <Blankslate.Heading>No dependencies detected</Blankslate.Heading>
          <Blankslate.Description>Reef reads Cargo.toml, package.json, go.mod, requirements.txt, pyproject.toml, pom.xml, build.gradle, composer.json, Gemfile and .csproj files on the default branch.</Blankslate.Description>
        </Blankslate>
      ) : (
        list.map((m) => (
          <section key={m.manifest} className="manifest">
            <header>
              <FileIcon />
              <Link to="/$owner/$repo/blob/$rev/$" params={{ ...p, rev: repo.default_branch, _splat: m.manifest }}>
                {m.manifest}
              </Link>
              <Label variant="secondary">{ECOSYSTEMS[m.ecosystem] ?? m.ecosystem}</Label>
              <span style={{ flex: 1 }} />
              <span className="muted small">
                {m.packages.length} {m.packages.length === 1 ? 'dependency' : 'dependencies'}
              </span>
            </header>
            {m.packages.length ? (
              <table className="dep-table">
                <thead>
                  <tr>
                    <th>Package</th>
                    <th>Requirement</th>
                    <th>Scope</th>
                  </tr>
                </thead>
                <tbody>
                  {m.packages.map((d, i) => (
                    <tr key={`${d.name}-${i}`}>
                      <td>
                        <PackageDependenciesIcon className="muted" /> <strong>{d.name}</strong>
                      </td>
                      <td>
                        <code>{d.requirement || '*'}</code>
                      </td>
                      <td>{d.kind === 'dev' ? <Label size="small">development</Label> : <span className="muted small">runtime</span>}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : (
              <p className="muted small" style={{ padding: '8px 16px', margin: 0 }}>
                No dependencies declared.
              </p>
            )}
          </section>
        ))
      )}
    </>
  );
}
