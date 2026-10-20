import { Blankslate } from '@primer/react/experimental';
import { SegmentedControl } from '@primer/react';
import { RepoIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { useState } from 'react';
import { CopyField, TreeView } from '../components/RepoCode';
import { RepoTitle } from '../components/RepoHeader';
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
    <>
      <RepoTitle repo={repo} />
      <div className="code-layout">
        <div>{empty ? <EmptyRepo repo={repo} /> : branches.data ? <TreeView repo={repo} rev={repo.default_branch} path="" /> : null}</div>
        <RepoSidebar repo={repo} empty={!!empty} />
      </div>
    </>
  );
}

/// GitHub's empty-repository page: the quick-setup box with the clone URL in HTTPS or SSH form,
/// and the two command-line recipes, which follow the chosen form.
function EmptyRepo({ repo }: { repo: Repo }) {
  const { instance } = useSession();
  const [proto, setProto] = useState<'https' | 'ssh'>('https');
  const https = `${instance?.site_url ?? location.origin}/${repo.full_name}.git`;
  const url = proto === 'ssh' ? repo.ssh_url : https;
  const canPush = repo.permissions?.push;
  if (!canPush) {
    return (
      <Blankslate border>
        <Blankslate.Visual>
          <RepoIcon size="medium" />
        </Blankslate.Visual>
        <Blankslate.Heading>This repository is empty</Blankslate.Heading>
        <Blankslate.Description>Nothing has been pushed yet.</Blankslate.Description>
      </Blankslate>
    );
  }
  const create = `echo "# ${repo.name}" >> README.md
git init
git add README.md
git commit -m "first commit"
git branch -M ${repo.default_branch}
git remote add origin ${url}
git push -u origin ${repo.default_branch}`;
  const push = `git remote add origin ${url}
git branch -M ${repo.default_branch}
git push -u origin ${repo.default_branch}`;
  return (
    <div className="empty-repo">
      <section className="quick-setup">
        <h2>Quick setup — if you’ve done this kind of thing before</h2>
        <div className="quick-setup-url">
          <SegmentedControl aria-label="Clone URL form" size="small" onChange={(i) => setProto(i === 1 ? 'ssh' : 'https')}>
            <SegmentedControl.Button selected={proto === 'https'}>HTTPS</SegmentedControl.Button>
            <SegmentedControl.Button selected={proto === 'ssh'}>SSH</SegmentedControl.Button>
          </SegmentedControl>
          <div className="quick-setup-field">
            <CopyField value={url} />
          </div>
        </div>
        <p>
          Get started by{' '}
          <Link to="/$owner/$repo/new/$rev/$" params={{ owner: repo.owner.login, repo: repo.name, rev: repo.default_branch, _splat: '' }}>
            creating a new file
          </Link>
          . We recommend every repository include a README, LICENSE, and .gitignore.
        </p>
      </section>
      <section>
        <h2>…or create a new repository on the command line</h2>
        <CommandBlock text={create} />
      </section>
      <section>
        <h2>…or push an existing repository from the command line</h2>
        <CommandBlock text={push} />
      </section>
      <p className="muted small">
        {proto === 'https' ? (
          <>
            Over HTTPS, authenticate with your username and a <Link to="/settings/tokens">personal access token</Link>.
          </>
        ) : (
          <>
            Over SSH, authenticate with a key registered under <Link to="/settings/keys">SSH and tokens</Link>.
          </>
        )}
      </p>
    </div>
  );
}

/// A command recipe with a copy button, as in GitHub's quick setup.
function CommandBlock({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="command-block">
      <pre>{text}</pre>
      <button
        type="button"
        className="copy"
        aria-label="Copy"
        onClick={() => {
          navigator.clipboard.writeText(text);
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        }}
      >
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  );
}

