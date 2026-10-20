// Blame, GitHub style: each hunk shows the commit summary, author and age on the left with
// a heat bar (newest is warmest), then line numbers and code.
import { Avatar, RelativeTime, SegmentedControl } from '@primer/react';
import { useQuery } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { decodeContent, humanSize, isBinary, type ContentFile, type Repo } from '../../lib/api';
import { blameQuery, contentsQuery, isMarkdown } from '../../lib/repo';
import { ErrorFlash, Loading } from '../Loading';
import { BranchMenu, PathBreadcrumbs } from '../RepoCode';
import { useLineSelection } from './BlobView';
import { langForPath, MAX_LINES } from '../../lib/highlight';
import { CodeText, useHighlight } from '../../lib/highlight/react';
import { usePanelToggle } from './CodeLayout';

export function BlameView({ repo, rev, path }: { repo: Repo; rev: string; path: string }) {
  const owner = repo.owner.login;
  const contents = useQuery(contentsQuery(owner, repo.name, rev, path));
  const blame = useQuery(blameQuery(owner, repo.name, rev, path));
  const [range, select] = useLineSelection();
  const navigate = useNavigate();
  const toggle = usePanelToggle();
  const md = isMarkdown(path);
  const p = { owner, repo: repo.name };
  const loadedFile = !contents.isPending && !contents.isError && !Array.isArray(contents.data) ? (contents.data as ContentFile) : null;
  const text = loadedFile ? decodeContent(loadedFile) : '';
  const lines = text.split('\n');
  if (lines[lines.length - 1] === '') lines.pop();
  const binary = loadedFile ? isBinary(text) : false;
  const tokens = useHighlight(text, langForPath(path), !!loadedFile && !binary && lines.length <= MAX_LINES);
  if (contents.isPending || blame.isPending) return <Loading />;
  if (contents.isError) return <ErrorFlash error={contents.error} />;
  if (blame.isError) return <ErrorFlash error={blame.error} />;
  if (Array.isArray(contents.data)) return null;
  const file = contents.data as ContentFile;
  const hunks = blame.data;
  // Age rank: 0 = newest commit in the file, 1 = oldest.
  const times = [...new Set(hunks.map((h) => new Date(h.date).getTime()))].sort((a, b) => b - a);
  const rank = (d: string) => (times.length <= 1 ? 0 : times.indexOf(new Date(d).getTime()) / (times.length - 1));
  return (
    <>
      <div className="code-toolbar file-path-row">
        {toggle}
        <BranchMenu repo={repo} rev={rev} path={path} mode="blob" />
        <PathBreadcrumbs repo={repo} rev={rev} path={path} mode="blob" />
      </div>
      <div className="blob blame">
        <div className="blob-header">
          <SegmentedControl
            aria-label="File view"
            size="small"
            onChange={(i) => {
              const last = md ? 2 : 1;
              if (i !== last) navigate({ to: '/$owner/$repo/blob/$rev/$', params: { ...p, rev, _splat: path } });
            }}
          >
            {md ? <SegmentedControl.Button selected={false}>Preview</SegmentedControl.Button> : null}
            <SegmentedControl.Button selected={false}>Code</SegmentedControl.Button>
            <SegmentedControl.Button selected>Blame</SegmentedControl.Button>
          </SegmentedControl>
          <span className="blob-meta">
            {lines.length} {lines.length === 1 ? 'line' : 'lines'} · {humanSize(file.size)} · {hunks.length} {hunks.length === 1 ? 'hunk' : 'hunks'}
          </span>
        </div>
        {binary ? (
          <p className="muted" style={{ padding: 32, textAlign: 'center' }}>
            Binary file: blame is not available.
          </p>
        ) : (
          <table className="code blame-table">
            {hunks.map((h) => {
              const [start, end] = h.lines;
              const n = end - start + 1;
              const slice = lines.slice(start - 1, end);
              return (
                <tbody key={`${h.sha}-${start}`} className="hunk">
                  {slice.map((l, i) => {
                    const ln = start + i;
                    const hl = range && ln >= range[0] && ln <= range[1];
                    return (
                      <tr key={ln} id={`L${ln}`} className={hl ? 'highlighted' : undefined}>
                        {i === 0 ? (
                          <td className="blame-meta" rowSpan={n} style={{ borderLeftColor: `rgba(255, 128, 0, ${(1 - rank(h.date)) * 0.9 + 0.1})` }}>
                            <div className="blame-commit">
                              <Link to="/$owner/$repo/commit/$sha" params={{ ...p, sha: h.sha }} className="blame-summary" title={h.summary}>
                                {h.summary || h.sha.slice(0, 7)}
                              </Link>
                              <span className="blame-author">
                                {h.author_login ? (
                                  <Link to="/$owner" params={{ owner: h.author_login }} title={h.author_login}>
                                    <Avatar src={h.avatar_url} size={16} alt="" />
                                  </Link>
                                ) : (
                                  <Avatar src={h.avatar_url} size={16} alt={h.author} />
                                )}
                                <RelativeTime datetime={h.date} />
                              </span>
                            </div>
                          </td>
                        ) : null}
                        <td className="ln">
                          <a
                            href={`#L${ln}`}
                            onClick={(e) => {
                              e.preventDefault();
                              select(ln, e.shiftKey);
                            }}
                          >
                            {ln}
                          </a>
                        </td>
                        <td className="code-cell">
                          <CodeText tokens={tokens?.[ln - 1]} text={l} />
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              );
            })}
          </table>
        )}
      </div>
    </>
  );
}
