// Unified and split diffs with a hover "+" gutter for inline comments, as on GitHub's
// "Files changed" tab. `under` lets the caller render review threads and comment forms below a line.
import { Checkbox } from '@primer/react';
import { ChevronDownIcon, ChevronRightIcon, CopyIcon, FileIcon, PlusIcon } from '@primer/octicons-react';
import { useMemo, useState } from 'react';
import { langForPath, type Token } from '../lib/highlight';
import { CodeText, useHighlight } from '../lib/highlight/react';
import { parsePatch, splitRows, type DiffLine, type PullFile } from '../lib/pulls';

export type Side = 'LEFT' | 'RIGHT';

/** The side and line number a comment on `l` attaches to (GitHub puts context lines on the right). */
export function lineAnchor(l: DiffLine): { side: Side; line: number } | null {
  if (l.type === 'del' && l.old !== undefined) return { side: 'LEFT', line: l.old };
  if (l.new !== undefined) return { side: 'RIGHT', line: l.new };
  return null;
}

export function DiffStat({ additions, deletions }: { additions: number; deletions: number }) {
  const total = additions + deletions;
  const blocks = total === 0 ? 0 : Math.min(5, Math.max(1, Math.round((total / Math.max(total, 1)) * 5)));
  const green = total === 0 ? 0 : Math.round((additions / total) * blocks);
  return (
    <span className="diffstat">
      <span className="add">+{additions}</span> <span className="del">−{deletions}</span>{' '}
      <span className="blocks">
        {Array.from({ length: 5 }).map((_, i) => (
          <span key={i} className={i < green ? 'g' : i < blocks ? 'r' : 'n'} />
        ))}
      </span>
    </span>
  );
}

export interface FileDiffProps {
  file: PullFile;
  defaultOpen?: boolean;
  split?: boolean;
  viewed?: boolean;
  onViewed?: (v: boolean) => void;
  canComment?: boolean;
  onAddComment?: (anchor: { side: Side; line: number }, l: DiffLine) => void;
  /** Rendered in a full-width row directly below the line (review threads, comment forms). */
  under?: (anchor: { side: Side; line: number }) => React.ReactNode;
}

function Gutter({ l, canComment, onAdd }: { l: DiffLine; canComment?: boolean; onAdd?: (a: { side: Side; line: number }, l: DiffLine) => void }) {
  const a = lineAnchor(l);
  if (!canComment || !a || !onAdd) return null;
  return (
    <button type="button" className="add-comment-btn" aria-label={`Add a comment on line ${a.line}`} onClick={() => onAdd(a, l)}>
      <PlusIcon size={12} />
    </button>
  );
}

export function FileDiff({ file, defaultOpen = true, split = false, viewed, onViewed, canComment, onAddComment, under }: FileDiffProps) {
  const [open, setOpen] = useState(defaultOpen);
  const [copied, setCopied] = useState(false);
  const lines = useMemo(() => parsePatch(file.patch), [file.patch]);
  // One tokenise call per file: every code line (hunk headers excluded) joined in order, then
  // mapped back to its DiffLine so both unified and split rows can look tokens up by identity.
  const codeLines = useMemo(() => lines.filter((l) => l.type !== 'hunk'), [lines]);
  const joined = useMemo(() => codeLines.map((l) => l.text).join('\n'), [codeLines]);
  const collapsed = !open || viewed;
  const highlighted = useHighlight(joined, langForPath(file.filename), !collapsed && !!file.patch && codeLines.length <= 5000);
  const tokensOf = useMemo(() => {
    const m = new Map<DiffLine, Token[]>();
    if (highlighted) codeLines.forEach((l, i) => m.set(l, highlighted[i] ?? []));
    return m;
  }, [highlighted, codeLines]);
  const underFor = (l: DiffLine | null) => {
    if (!l || !under) return null;
    const a = lineAnchor(l);
    return a ? under(a) : null;
  };
  return (
    <div className={`file-diff${viewed ? ' viewed' : ''}`} id={`diff-${file.filename}`}>
      <div className="file-diff-header">
        <button className="linkish chev" onClick={() => setOpen(!open)} aria-label={open ? 'Collapse' : 'Expand'}>
          {collapsed ? <ChevronRightIcon /> : <ChevronDownIcon />}
        </button>
        <FileIcon className="muted" />
        <span className="path monospace">
          {file.previous_filename ? (
            <>
              {file.previous_filename} → {file.filename}
            </>
          ) : (
            file.filename
          )}
        </span>
        <button
          className="linkish muted copy"
          aria-label="Copy path"
          title={copied ? 'Copied' : 'Copy path'}
          onClick={() => {
            navigator.clipboard?.writeText(file.filename);
            setCopied(true);
            setTimeout(() => setCopied(false), 1200);
          }}
        >
          <CopyIcon size={14} />
        </button>
        {file.status !== 'modified' ? <span className={`status-pill ${file.status}`}>{file.status}</span> : null}
        <span className="grow" />
        <DiffStat additions={file.additions} deletions={file.deletions} />
        {onViewed ? (
          <label className="viewed-toggle">
            <Checkbox checked={!!viewed} onChange={(e) => onViewed(e.target.checked)} /> Viewed
          </label>
        ) : null}
      </div>
      {collapsed ? null : !file.patch ? (
        <p className="muted" style={{ padding: 16, margin: 0 }}>
          Binary file or no textual changes.
        </p>
      ) : split ? (
        <table className="diff-table split">
          <tbody>
            {splitRows(lines).map((r, i) =>
              r.hunk ? (
                <tr key={i} className="hunk">
                  <td className="ln" />
                  <td className="code" colSpan={3}>
                    {r.hunk.text}
                  </td>
                </tr>
              ) : (
                <RowGroup key={i} extra={[underFor(r.left), r.left === r.right ? null : underFor(r.right)]}>
                  <tr>
                    <td className={`ln ${r.left?.type ?? 'empty'}`}>
                      {r.left ? <Gutter l={r.left} canComment={canComment} onAdd={onAddComment} /> : null}
                      {r.left?.old ?? ''}
                    </td>
                    <td className={`code ${r.left?.type ?? 'empty'}`}>
                      {r.left ? (
                        <>
                          <span className="marker">{r.left.type === 'del' ? '-' : ' '}</span>
                          <CodeText tokens={tokensOf.get(r.left)} text={r.left.text} />
                        </>
                      ) : null}
                    </td>
                    <td className={`ln ${r.right?.type ?? 'empty'}`}>
                      {r.right && r.right !== r.left ? <Gutter l={r.right} canComment={canComment} onAdd={onAddComment} /> : r.right ? null : null}
                      {r.right?.new ?? ''}
                    </td>
                    <td className={`code ${r.right?.type ?? 'empty'}`}>
                      {r.right ? (
                        <>
                          <span className="marker">{r.right.type === 'add' ? '+' : ' '}</span>
                          <CodeText tokens={tokensOf.get(r.right)} text={r.right.text} />
                        </>
                      ) : null}
                    </td>
                  </tr>
                </RowGroup>
              ),
            )}
          </tbody>
        </table>
      ) : (
        <table className="diff-table">
          <tbody>
            {lines.map((l, i) =>
              l.type === 'hunk' ? (
                <tr key={i} className="hunk">
                  <td className="ln" colSpan={2} />
                  <td className="code">{l.text}</td>
                </tr>
              ) : (
                <RowGroup key={i} extra={[underFor(l)]}>
                  <tr className={l.type}>
                    <td className="ln">
                      <Gutter l={l} canComment={canComment} onAdd={onAddComment} />
                      {l.old ?? ''}
                    </td>
                    <td className="ln">{l.new ?? ''}</td>
                    <td className="code">
                      <span className="marker">{l.type === 'add' ? '+' : l.type === 'del' ? '-' : ' '}</span>
                      <CodeText tokens={tokensOf.get(l)} text={l.text} />
                    </td>
                  </tr>
                </RowGroup>
              ),
            )}
          </tbody>
        </table>
      )}
    </div>
  );
}

/** A diff row plus any full-width rows (threads, forms) that hang under it. */
function RowGroup({ children, extra }: { children: React.ReactNode; extra: React.ReactNode[] }) {
  const items = extra.filter(Boolean);
  return (
    <>
      {children}
      {items.map((e, i) => (
        <tr key={i} className="inline-row">
          <td colSpan={4}>{e}</td>
        </tr>
      ))}
    </>
  );
}

export function FilesChanged({ files }: { files: PullFile[] }) {
  const adds = files.reduce((a, f) => a + f.additions, 0);
  const dels = files.reduce((a, f) => a + f.deletions, 0);
  return (
    <div>
      <div className="muted" style={{ margin: '8px 0 16px' }}>
        Showing <strong>{files.length}</strong> changed {files.length === 1 ? 'file' : 'files'} with <span className="add">{adds} additions</span> and <span className="del">{dels} deletions</span>.
      </div>
      {files.map((f) => (
        <FileDiff key={f.filename} file={f} defaultOpen={files.length <= 20} />
      ))}
    </div>
  );
}
