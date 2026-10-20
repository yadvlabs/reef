// Create, edit, or delete a file in the browser; commits directly to a branch.
import { Button, FormControl, Textarea, TextInput } from '@primer/react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { api, type ContentFile, type Repo, decodeContent } from '../lib/api';
import { contentsQuery } from '../lib/repo';
import { ErrorFlash, Loading } from './Loading';
import { PathBreadcrumbs } from './RepoCode';

export function FileEditor({ repo, rev, path, mode }: { repo: Repo; rev: string; path: string; mode: 'new' | 'edit' | 'delete' }) {
  const owner = repo.owner.login;
  const qc = useQueryClient();
  const navigate = useNavigate();
  const existing = useQuery({ ...contentsQuery(owner, repo.name, rev, path), enabled: mode !== 'new' });
  const dir = mode === 'new' ? path : path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
  const [name, setName] = useState(mode === 'new' ? '' : path.slice(dir ? dir.length + 1 : 0));
  const [text, setText] = useState<string | null>(null);
  const [message, setMessage] = useState('');
  const [description, setDescription] = useState('');
  const file = existing.data && !Array.isArray(existing.data) ? (existing.data as ContentFile) : null;
  const content = text ?? (file ? decodeContent(file) : '');
  const fullPath = dir ? `${dir}/${name}` : name;
  const defaultMessage = mode === 'delete' ? `Delete ${path}` : mode === 'new' ? (name.trim() ? `Create ${fullPath}` : 'Create new file') : `Update ${path}`;
  const commit = useMutation({
    mutationFn: async () => {
      const msg = [message.trim() || defaultMessage, description.trim()].filter(Boolean).join('\n\n');
      const target = `/repos/${repo.full_name}/contents/${(mode === 'new' ? fullPath : path).split('/').map(encodeURIComponent).join('/')}`;
      if (mode === 'delete') return api(target, { method: 'DELETE', json: { message: msg, sha: file?.sha, branch: rev } });
      const bytes = new TextEncoder().encode(content);
      let bin = '';
      bytes.forEach((b) => (bin += String.fromCharCode(b)));
      return api(target, { method: 'PUT', json: { message: msg, content: btoa(bin), sha: mode === 'edit' ? file?.sha : undefined, branch: rev } });
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['repo', owner.toLowerCase(), repo.name.toLowerCase()] });
      if (mode === 'delete') navigate({ to: '/$owner/$repo/tree/$rev/$', params: { owner, repo: repo.name, rev, _splat: dir } });
      else navigate({ to: '/$owner/$repo/blob/$rev/$', params: { owner, repo: repo.name, rev, _splat: mode === 'new' ? fullPath : path } });
    },
  });
  if (mode !== 'new' && existing.isPending) return <Loading />;
  if (mode !== 'new' && existing.isError) return <ErrorFlash error={existing.error} />;
  const lines = content.split('\n').length;
  return (
    <form
      className="form-stack file-editor"
      onSubmit={(e) => {
        e.preventDefault();
        commit.mutate();
      }}
    >
      <div className="code-toolbar file-path-row" style={{ marginBottom: 0 }}>
        <PathBreadcrumbs repo={repo} rev={rev} path={dir} mode="tree" />
        {mode !== 'delete' ? (
          <>
            <span className="muted">/</span>
            <TextInput size="small" value={name} onChange={(e) => setName(e.target.value)} placeholder="Name your file..." aria-label="File name" autoFocus={mode === 'new'} disabled={mode === 'edit'} style={{ width: 260 }} />
          </>
        ) : (
          <strong>{name}</strong>
        )}
        <span className="muted small" style={{ marginLeft: 8 }}>
          in <code>{rev}</code>
        </span>
      </div>
      {commit.isError ? <ErrorFlash error={commit.error} /> : null}
      {mode === 'delete' ? (
        <div className="card">
          <p style={{ margin: 0 }}>
            This will delete <strong>{path}</strong> from <code>{rev}</code> in a new commit.
          </p>
        </div>
      ) : (
        <div className="blob">
          <div className="blob-header">
            <span>{lines} lines</span>
            <span className="spacer" />
            <span className="muted small">Plain text editor</span>
          </div>
          <textarea className="code-editor" value={content} onChange={(e) => setText(e.target.value)} spellCheck={false} aria-label="File contents" rows={Math.max(18, Math.min(lines + 2, 60))} />
        </div>
      )}
      <div className="card form-stack commit-box">
        <h3 style={{ margin: 0, fontSize: 16 }}>Commit changes</h3>
        <FormControl>
          <FormControl.Label>Commit message</FormControl.Label>
          <TextInput block value={message} onChange={(e) => setMessage(e.target.value)} placeholder={defaultMessage} />
        </FormControl>
        <FormControl>
          <FormControl.Label>Extended description</FormControl.Label>
          <Textarea block rows={3} value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Add an optional extended description..." />
        </FormControl>
        <div style={{ display: 'flex', gap: 8 }}>
          <Button type="submit" variant={mode === 'delete' ? 'danger' : 'primary'} loading={commit.isPending} disabled={mode !== 'delete' && !name.trim()}>
            {mode === 'delete' ? 'Commit deletion' : 'Commit changes'}
          </Button>
          <Button type="button" onClick={() => history.back()}>
            Cancel
          </Button>
          <span className="muted small" style={{ alignSelf: 'center' }}>
            Commits directly to the <code>{rev}</code> branch.
          </span>
        </div>
      </div>
    </form>
  );
}
