// Wiki pages shared by repository wikis (/owner/repo/wiki) and organization wikis (/org/wiki).
import { Button, FormControl, TextInput, RelativeTime } from '@primer/react';
import { PencilIcon, HistoryIcon, PlusIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { Crusty } from './Crusty';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { MarkdownEditor } from './IssueBits';
import { ErrorFlash, Loading } from './Loading';
import { Markdown } from './Markdown';
import { reef, type Repo } from '../lib/api';
import { useSession } from '../lib/session';

export interface WikiIndex {
  head: string | null;
  clone_url: string;
  pages: { title: string; file: string; size: number }[];
}
export interface WikiPage {
  title: string;
  file: string;
  head: string;
  content: string;
  html: string;
  history: { sha: string; message: string; author: string; date: string }[];
}

export interface WikiProps {
  /// Reef API base, e.g. `/repos/acme/app/wiki` or `/orgs/acme/wiki`.
  apiBase: string;
  /// Web path base, e.g. `/acme/app/wiki` or `/acme/wiki`.
  webBase: string;
  /// The path after the base: `Page`, `Page/_edit`, `Page/_history`, `_new`.
  splat: string;
  title: string;
  canEdit: boolean;
  /// Repository for markdown preview context (repository wikis only).
  repo?: Repo;
}

function WikiLink({ href, children, className, style }: { href: string; children: React.ReactNode; className?: string; style?: React.CSSProperties }) {
  const navigate = useNavigate();
  return (
    <a
      href={href}
      className={className}
      style={style}
      onClick={(e) => {
        e.preventDefault();
        navigate({ href });
      }}
    >
      {children}
    </a>
  );
}

const indexQuery = (apiBase: string) => ({ queryKey: ['wiki', apiBase], queryFn: () => reef<WikiIndex>(apiBase) });
const pageQuery = (apiBase: string, page: string, history = false) => ({ queryKey: ['wiki', apiBase, page, history], queryFn: () => reef<WikiPage>(`${apiBase}/${encodeURIComponent(page)}${history ? '?history=1' : ''}`) });

export function WikiPages(props: WikiProps) {
  const { apiBase, webBase, splat, title, canEdit } = props;
  const parts = splat.split('/').filter(Boolean).map(decodeURIComponent);
  const [page, action] = parts[0] === '_new' ? ['', 'new'] : [parts[0] ?? '', parts[1] ?? ''];
  const index = useQuery(indexQuery(apiBase));
  if (index.isPending) return <Loading />;
  if (index.isError) return <ErrorFlash error={index.error} />;
  const home = index.data.pages.find((x) => x.file.toLowerCase() === 'home.md');
  const current = page || (home ? home.title : '');
  if (action === 'new') return <Editor {...props} head={index.data.head ?? ''} />;
  if (!current) {
    return (
      <Blankslate spacious>
        <Blankslate.Visual>
          <Crusty mood="builder" size={120} />
        </Blankslate.Visual>
        <Blankslate.Heading>Welcome to the {title} wiki!</Blankslate.Heading>
        <Blankslate.Description>Wikis are a place to lay out the roadmap of your project, show the current status, and document software better, together.</Blankslate.Description>
        {canEdit ? <Blankslate.PrimaryAction href={`${webBase}/_new`}>Create the first page</Blankslate.PrimaryAction> : null}
      </Blankslate>
    );
  }
  return (
    <div className="code-layout">
      <div>{action === '_edit' ? <Editor {...props} page={current} head={index.data.head ?? ''} /> : <PageView {...props} page={current} history={action === '_history'} />}</div>
      <aside className="about">
        <h2 style={{ display: 'flex', alignItems: 'center' }}>
          Pages <span style={{ flex: 1 }} />
          {canEdit ? (
            <Button as="a" size="small" leadingVisual={PlusIcon} {...({ href: `${webBase}/_new` } as object)}>
              New page
            </Button>
          ) : null}
        </h2>
        <div className="meta">
          {index.data.pages.map((pg) => (
            <WikiLink key={pg.file} href={`${webBase}/${encodeURIComponent(pg.title)}`} style={{ color: pg.title === current ? 'var(--fgColor-default)' : undefined, fontWeight: pg.title === current ? 600 : 400 }}>
              {pg.title}
            </WikiLink>
          ))}
        </div>
        <h2 style={{ marginTop: 24 }}>Clone this wiki locally</h2>
        <TextInput readOnly value={index.data.clone_url} block monospace size="small" aria-label="Wiki clone URL" />
      </aside>
    </div>
  );
}

function PageView({ apiBase, webBase, page, history, canEdit }: WikiProps & { page: string; history: boolean }) {
  const q = useQuery(pageQuery(apiBase, page, history));
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const pg = q.data;
  const enc = encodeURIComponent(pg.title);
  return (
    <div>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, borderBottom: '1px solid var(--borderColor-default)', paddingBottom: 8, marginBottom: 16 }}>
        <h1 style={{ fontSize: 24, fontWeight: 400, margin: 0, flex: 1 }}>{pg.title}</h1>
        <Button as="a" size="small" leadingVisual={HistoryIcon} {...({ href: `${webBase}/${enc}/_history` } as object)}>
          History
        </Button>
        {canEdit ? (
          <Button as="a" size="small" leadingVisual={PencilIcon} {...({ href: `${webBase}/${enc}/_edit` } as object)}>
            Edit
          </Button>
        ) : null}
      </div>
      {history ? (
        <div className="commit-list">
          <div className="group">
            {pg.history.map((h) => (
              <div key={h.sha} className="commit">
                <div className="body">
                  <span className="title">{h.message}</span>
                  <div className="who">
                    <strong>{h.author}</strong> committed <RelativeTime datetime={h.date} />
                  </div>
                </div>
                <code className="sha">{h.sha.slice(0, 7)}</code>
              </div>
            ))}
          </div>
        </div>
      ) : (
        <Markdown html={pg.html} />
      )}
    </div>
  );
}

function Editor({ apiBase, webBase, page, head, repo }: WikiProps & { page?: string; head: string }) {
  const { user } = useSession();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const existing = useQuery({ ...pageQuery(apiBase, page ?? ''), enabled: !!page });
  const [title, setTitle] = useState(page ?? '');
  const [content, setContent] = useState<string | null>(null);
  const [message, setMessage] = useState('');
  const body = content ?? existing.data?.content ?? '';
  const save = useMutation({
    mutationFn: () =>
      page
        ? reef<{ title: string }>(`${apiBase}/${encodeURIComponent(page)}`, { method: 'PUT', json: { title, content: body, message, head: existing.data?.head ?? head } })
        : reef<{ title: string }>(apiBase, { method: 'POST', json: { title, content: body, message, head } }),
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: ['wiki', apiBase] });
      navigate({ href: `${webBase}/${encodeURIComponent(r.title)}` });
    },
  });
  const del = useMutation({
    mutationFn: () => reef(`${apiBase}/${encodeURIComponent(page!)}?head=${existing.data?.head ?? head}`, { method: 'DELETE' }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['wiki', apiBase] });
      navigate({ href: webBase });
    },
  });
  if (!user) {
    navigate({ to: '/login', search: { return_to: location.pathname } });
    return null;
  }
  if (page && existing.isPending) return <Loading />;
  return (
    <form
      className="form-stack"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
    >
      <h1 style={{ fontSize: 24, fontWeight: 400, margin: 0 }}>{page ? `Editing ${page}` : 'Create new page'}</h1>
      {save.isError ? <ErrorFlash error={save.error} /> : null}
      {del.isError ? <ErrorFlash error={del.error} /> : null}
      <FormControl required>
        <FormControl.Label>Title</FormControl.Label>
        <TextInput block value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Page title" />
      </FormControl>
      <FormControl>
        <FormControl.Label>Content</FormControl.Label>
        <MarkdownEditor value={body} onChange={setContent} repo={repo} minHeight={300} />
      </FormControl>
      <FormControl>
        <FormControl.Label>Edit message</FormControl.Label>
        <TextInput block value={message} onChange={(e) => setMessage(e.target.value)} placeholder={page ? `Update ${page}` : `Create ${title || 'page'}`} />
      </FormControl>
      <div style={{ display: 'flex', gap: 8 }}>
        <Button type="submit" variant="primary" disabled={!title.trim()} loading={save.isPending}>
          Save page
        </Button>
        <Button type="button" onClick={() => history.back()}>
          Cancel
        </Button>
        <span style={{ flex: 1 }} />
        {page ? (
          <Button type="button" variant="danger" onClick={() => confirm(`Delete the page "${page}"?`) && del.mutate()}>
            Delete page
          </Button>
        ) : null}
      </div>
    </form>
  );
}
