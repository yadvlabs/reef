// A file, GitHub style: breadcrumb toolbar, a box header with Preview/Code/Blame, line and
// size metadata, Raw/Copy/Download, Edit, and a "more" menu; line numbers are permalinks
// (`#L10`, `#L10-L20`) and the selected range is highlighted.
import { LfsPointer, parseLfsPointer } from '../mirror/LfsPointer';
import { ActionList, ActionMenu, Button, ButtonGroup, IconButton, SegmentedControl } from '@primer/react';
import { CheckIcon, CopyIcon, DownloadIcon, HistoryIcon, KebabHorizontalIcon, LinkExternalIcon, PencilIcon, TrashIcon } from '@primer/octicons-react';
import { useQuery } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { decodeContent, humanSize, isBinary, type ContentFile, type Repo } from '../../lib/api';
import { contentsQuery, fileHtmlQuery, isMarkdown, rawUrl } from '../../lib/repo';
import { langForPath, MAX_LINES, type Lines } from '../../lib/highlight';
import { CodeText, useHighlight } from '../../lib/highlight/react';
import { ActionListLink } from '../links';
import { ErrorFlash, Loading } from '../Loading';
import { Markdown } from '../Markdown';
import { BranchMenu, PathBreadcrumbs, TreeView } from '../RepoCode';
import { usePanelToggle } from './CodeLayout';

/** `#L10` or `#L10-L20` → [10, 20]. */
export function parseLineHash(hash: string): [number, number] | null {
  const m = /^#L(\d+)(?:-L(\d+))?$/.exec(hash);
  if (!m) return null;
  const a = Number(m[1]);
  const b = m[2] ? Number(m[2]) : a;
  return [Math.min(a, b), Math.max(a, b)];
}

export function useLineSelection(): [[number, number] | null, (line: number, extend: boolean) => void] {
  const [range, setRange] = useState<[number, number] | null>(() => parseLineHash(location.hash));
  useEffect(() => {
    const onHash = () => setRange(parseLineHash(location.hash));
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  }, []);
  useEffect(() => {
    if (!range) return;
    const el = document.getElementById(`L${range[0]}`);
    if (el) el.scrollIntoView({ block: 'center' });
    // only on first paint / navigation, not on every click
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const select = (line: number, extend: boolean) => {
    const next: [number, number] = extend && range ? [Math.min(range[0], line), Math.max(range[1], line)] : [line, line];
    const hash = next[0] === next[1] ? `#L${next[0]}` : `#L${next[0]}-L${next[1]}`;
    history.replaceState(history.state, '', `${location.pathname}${location.search}${hash}`);
    setRange(next);
  };
  return [range, select];
}

/** Line numbers plus code, shared by the code and blame views. */
export function CodeLines({ lines, range, onSelect, offset = 0, tokens }: { lines: string[]; range: [number, number] | null; onSelect: (line: number, extend: boolean) => void; offset?: number; tokens?: Lines | null }) {
  return (
    <>
      {lines.map((l, i) => {
        const n = offset + i + 1;
        const hl = range && n >= range[0] && n <= range[1];
        return (
          <tr key={n} id={`L${n}`} className={hl ? 'highlighted' : undefined}>
            <td className="ln">
              <a
                href={`#L${n}`}
                onClick={(e) => {
                  e.preventDefault();
                  onSelect(n, e.shiftKey);
                }}
              >
                {n}
              </a>
            </td>
            <td className="code-cell">
              <CodeText tokens={tokens?.[offset + i]} text={l} />
            </td>
          </tr>
        );
      })}
    </>
  );
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <IconButton
      icon={copied ? CheckIcon : CopyIcon}
      aria-label="Copy raw file"
      size="small"
      onClick={() => {
        navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
    />
  );
}

export function BlobView({ repo, rev, path }: { repo: Repo; rev: string; path: string }) {
  const owner = repo.owner.login;
  const contents = useQuery(contentsQuery(owner, repo.name, rev, path));
  const md = isMarkdown(path);
  const html = useQuery({ ...fileHtmlQuery(owner, repo.name, rev, path), enabled: md });
  const [mode, setMode] = useState<'preview' | 'code'>(md ? 'preview' : 'code');
  const [range, select] = useLineSelection();
  const navigate = useNavigate();
  const toggle = usePanelToggle();
  // Hooks run before the early returns: decode whatever is loaded so far.
  const loadedFile = !contents.isPending && !contents.isError && !Array.isArray(contents.data) ? (contents.data as ContentFile) : null;
  const text = loadedFile ? decodeContent(loadedFile) : '';
  const binary = loadedFile ? isBinary(text) : false;
  const lines = text.split('\n');
  if (lines[lines.length - 1] === '') lines.pop();
  const tokens = useHighlight(text, langForPath(path), !!loadedFile && !binary && lines.length <= MAX_LINES && !(md && mode === 'preview'));
  if (contents.isPending) return <Loading />;
  if (contents.isError) return <ErrorFlash error={contents.error} />;
  if (Array.isArray(contents.data)) return <TreeView repo={repo} rev={rev} path={path} />;
  const file = contents.data as ContentFile;
  const lfs = !binary ? parseLfsPointer(text) : null;
  if (lfs) return <LfsPointer repo={repo} path={path} pointer={lfs} />;
  const loc = lines.filter((l) => l.trim() !== '').length;
  const raw = rawUrl(repo.full_name, rev, path);
  const p = { owner, repo: repo.name };
  const segIndex = md ? (mode === 'preview' ? 0 : 1) : 0;
  return (
    <>
      <div className="code-toolbar file-path-row">
        {toggle}
        <BranchMenu repo={repo} rev={rev} path={path} mode="blob" />
        <PathBreadcrumbs repo={repo} rev={rev} path={path} mode="blob" />
        <CopyPath path={path} />
      </div>
      <div className="blob">
        <div className="blob-header">
          <SegmentedControl
            aria-label="File view"
            size="small"
            onChange={(i) => {
              const labels = md ? ['preview', 'code', 'blame'] : ['code', 'blame'];
              const pick = labels[i];
              if (pick === 'blame') navigate({ to: '/$owner/$repo/blame/$rev/$', params: { ...p, rev, _splat: path } });
              else setMode(pick as 'preview' | 'code');
            }}
          >
            {md ? <SegmentedControl.Button selected={segIndex === 0}>Preview</SegmentedControl.Button> : null}
            <SegmentedControl.Button selected={md ? segIndex === 1 : true}>Code</SegmentedControl.Button>
            <SegmentedControl.Button selected={false}>Blame</SegmentedControl.Button>
          </SegmentedControl>
          <span className="blob-meta">
            {binary ? humanSize(file.size) : `${lines.length} ${lines.length === 1 ? 'line' : 'lines'} (${loc} loc) · ${humanSize(file.size)}`}
          </span>
          <span className="spacer" />
          <ButtonGroup>
            <Button as="a" href={raw} size="small" {...({ target: '_blank', rel: 'noreferrer' } as object)}>
              Raw
            </Button>
            <CopyButton text={text} />
            <IconButton as="a" href={raw} icon={DownloadIcon} aria-label="Download raw file" size="small" {...({ download: file.name } as object)} />
          </ButtonGroup>
          {repo.permissions?.push && !binary ? (
            <IconButton icon={PencilIcon} aria-label="Edit this file" size="small" onClick={() => navigate({ to: '/$owner/$repo/edit/$rev/$', params: { ...p, rev, _splat: path } })} />
          ) : null}
          <ActionMenu>
            <ActionMenu.Anchor>
              <IconButton icon={KebabHorizontalIcon} aria-label="More file actions" size="small" />
            </ActionMenu.Anchor>
            <ActionMenu.Overlay align="end">
              <ActionList>
                <ActionListLink to="/$owner/$repo/commits/$rev" params={{ ...p, rev }} search={{ path }}>
                  <ActionList.LeadingVisual>
                    <HistoryIcon />
                  </ActionList.LeadingVisual>
                  View history
                </ActionListLink>
                <ActionList.LinkItem href={raw} target="_blank" rel="noreferrer">
                  <ActionList.LeadingVisual>
                    <LinkExternalIcon />
                  </ActionList.LeadingVisual>
                  Open raw file
                </ActionList.LinkItem>
                {repo.permissions?.push ? (
                  <>
                    <ActionList.Divider />
                    <ActionListLink to="/$owner/$repo/edit/$rev/$" params={{ ...p, rev, _splat: path }} search={{ delete: true }}>
                      <ActionList.LeadingVisual>
                        <TrashIcon />
                      </ActionList.LeadingVisual>
                      Delete file
                    </ActionListLink>
                  </>
                ) : null}
              </ActionList>
            </ActionMenu.Overlay>
          </ActionMenu>
        </div>
        {md && mode === 'preview' ? (
          html.data ? (
            <Markdown html={html.data} />
          ) : (
            <Loading />
          )
        ) : binary ? (
          <p className="muted" style={{ padding: 32, textAlign: 'center' }}>
            Binary file not shown. <a href={raw}>View raw</a>
          </p>
        ) : (
          <table className="code">
            <tbody>
              <CodeLines lines={lines} range={range} onSelect={select} tokens={tokens} />
            </tbody>
          </table>
        )}
      </div>
    </>
  );
}

function CopyPath({ path }: { path: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <IconButton
      icon={copied ? CheckIcon : CopyIcon}
      aria-label="Copy path"
      size="small"
      variant="invisible"
      onClick={() => {
        navigator.clipboard.writeText(path);
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      }}
    />
  );
}
