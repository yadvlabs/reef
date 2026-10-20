import { ActionList, ActionMenu, Button, Checkbox, FormControl, TextInput } from '@primer/react';
import { TagIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, useNavigate } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { MarkdownEditor } from '../components/IssueBits';
import { ErrorFlash } from '../components/Loading';
import { api, type Release } from '../lib/api';
import { branchesQuery, tagsQuery } from '../lib/repo';
import { useRepo } from './$owner.$repo';

export const Route = createFileRoute('/$owner/$repo/releases/new')({
  validateSearch: (s: Record<string, unknown>): { tag?: string; edit?: number } => ({ ...(typeof s.tag === 'string' ? { tag: s.tag } : {}), ...(typeof s.edit === 'number' ? { edit: s.edit } : {}) }),
  component: NewRelease,
});

function NewRelease() {
  const repo = useRepo();
  const { tag: initialTag, edit } = Route.useSearch();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const tags = useQuery(tagsQuery(repo.owner.login, repo.name));
  const branches = useQuery(branchesQuery(repo.owner.login, repo.name));
  const existing = useQuery({ queryKey: ['release', edit], queryFn: () => api<Release>(`/repos/${repo.full_name}/releases/${edit}`), enabled: !!edit });
  const [tag, setTag] = useState(initialTag ?? '');
  const [target, setTarget] = useState(repo.default_branch);
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [prerelease, setPrerelease] = useState(false);
  const [tagFilter, setTagFilter] = useState('');
  useEffect(() => {
    if (existing.data) {
      setTag(existing.data.tag_name);
      setTarget(existing.data.target_commitish);
      setTitle(existing.data.name ?? '');
      setBody(existing.data.body ?? '');
      setPrerelease(existing.data.prerelease);
    }
  }, [existing.data]);
  const p = { owner: repo.owner.login, repo: repo.name };
  const save = useMutation({
    mutationFn: (draft: boolean) =>
      edit
        ? api<Release>(`/repos/${repo.full_name}/releases/${edit}`, { method: 'PATCH', json: { tag_name: tag, target_commitish: target, name: title, body, draft, prerelease } })
        : api<Release>(`/repos/${repo.full_name}/releases`, { method: 'POST', json: { tag_name: tag, target_commitish: target, name: title, body, draft, prerelease } }),
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: ['repo', repo.owner.login.toLowerCase(), repo.name.toLowerCase()] });
      navigate({ to: '/$owner/$repo/releases/tag/$tag', params: { ...p, tag: r.tag_name } });
    },
  });
  const generate = useMutation({
    mutationFn: async () => {
      // Ask the server for generated notes by creating a draft, reading it, then deleting it.
      const d = await api<Release>(`/repos/${repo.full_name}/releases`, { method: 'POST', json: { tag_name: tag || `draft-${Date.now()}`, target_commitish: target, draft: true, generate_release_notes: true } });
      await api(`/repos/${repo.full_name}/releases/${d.id}`, { method: 'DELETE' });
      return d.body ?? '';
    },
    onSuccess: (notes) => setBody((b) => (b.trim() ? `${b}\n\n${notes}` : notes)),
  });
  const tagExists = tags.data?.some((t) => t.name === tag);
  const tagList = (tags.data ?? []).filter((t) => t.name.includes(tagFilter)).slice(0, 30);
  return (
    <form
      className="form-stack narrow-form"
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate(false);
      }}
    >
      <h1 style={{ fontSize: 24, fontWeight: 400, margin: 0 }}>{edit ? 'Edit release' : 'New release'}</h1>
      {save.isError ? <ErrorFlash error={save.error} /> : null}
      {generate.isError ? <ErrorFlash error={generate.error} /> : null}
      <div style={{ display: 'flex', gap: 8, alignItems: 'flex-start', flexWrap: 'wrap' }}>
        <ActionMenu>
          <ActionMenu.Button leadingVisual={TagIcon}>{tag ? tag : 'Choose a tag'}</ActionMenu.Button>
          <ActionMenu.Overlay width="medium">
            <div style={{ padding: 8 }}>
              <TextInput block size="small" placeholder="Find or create a new tag" value={tagFilter} onChange={(e) => setTagFilter(e.target.value)} autoFocus />
            </div>
            <ActionList selectionVariant="single">
              {tagFilter && !tags.data?.some((t) => t.name === tagFilter) ? (
                <ActionList.Item onSelect={() => setTag(tagFilter)}>
                  <ActionList.LeadingVisual>
                    <TagIcon />
                  </ActionList.LeadingVisual>
                  Create new tag: {tagFilter} on publish
                </ActionList.Item>
              ) : null}
              {tagList.map((t) => (
                <ActionList.Item key={t.name} selected={t.name === tag} onSelect={() => setTag(t.name)}>
                  {t.name}
                </ActionList.Item>
              ))}
            </ActionList>
          </ActionMenu.Overlay>
        </ActionMenu>
        {!tagExists ? (
          <ActionMenu>
            <ActionMenu.Button>Target: {target}</ActionMenu.Button>
            <ActionMenu.Overlay>
              <ActionList selectionVariant="single">
                {(branches.data ?? []).map((b) => (
                  <ActionList.Item key={b.name} selected={b.name === target} onSelect={() => setTarget(b.name)}>
                    {b.name}
                  </ActionList.Item>
                ))}
              </ActionList>
            </ActionMenu.Overlay>
          </ActionMenu>
        ) : (
          <span className="muted small" style={{ alignSelf: 'center' }}>Existing tag</span>
        )}
      </div>
      {!tag ? <p className="muted small" style={{ margin: 0 }}>Choose an existing tag, or type a new one to create on publish.</p> : null}
      <FormControl>
        <FormControl.Label>Release title</FormControl.Label>
        <TextInput block value={title} onChange={(e) => setTitle(e.target.value)} placeholder={tag || 'Release title'} />
      </FormControl>
      <FormControl>
        <FormControl.Label>Describe this release</FormControl.Label>
        <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: 4 }}>
          <Button size="small" onClick={() => generate.mutate()} loading={generate.isPending} disabled={!tag}>
            Generate release notes
          </Button>
        </div>
        <MarkdownEditor value={body} onChange={setBody} repo={repo} minHeight={200} placeholder="Describe this release" />
      </FormControl>
      <FormControl>
        <Checkbox checked={prerelease} onChange={(e) => setPrerelease(e.target.checked)} />
        <FormControl.Label>Set as a pre-release</FormControl.Label>
        <FormControl.Caption>This release is labelled as non-production ready.</FormControl.Caption>
      </FormControl>
      <div style={{ display: 'flex', gap: 8 }}>
        <Button type="submit" variant="primary" disabled={!tag} loading={save.isPending}>
          {edit ? 'Update release' : 'Publish release'}
        </Button>
        {!edit ? (
          <Button type="button" disabled={!tag} onClick={() => save.mutate(true)}>
            Save draft
          </Button>
        ) : null}
        <Button type="button" onClick={() => history.back()}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
