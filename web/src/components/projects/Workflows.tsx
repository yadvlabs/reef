// Project workflows: GitHub's default automations with an on/off switch and a value to set.
import { Button, FormControl, NavList, Select, TextInput, ToggleSwitch } from '@primer/react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { reef } from '../../lib/api';
import { OPTION_DOTS, WORKFLOW_META, workflowsQuery, type ProjectFull, type ProjectWorkflow, type WorkflowKind } from '../../lib/projects';
import { ErrorFlash, Loading } from '../Loading';

export function Workflows({ project }: { project: ProjectFull }) {
  const q = useQuery(workflowsQuery(project.owner.login, project.number));
  const [selected, setSelected] = useState<WorkflowKind>('item_added');
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const list = q.data.workflows;
  const current = list.find((w) => w.kind === selected) ?? list[0];
  return (
    <div className="page workflows-layout">
      <aside>
        <h2 className="section-title">Workflows</h2>
        <NavList aria-label="Workflows">
          <NavList.Group title="Default workflows">
            {list.map((w) => (
              <NavList.Item key={w.kind} aria-current={w.kind === current.kind ? 'page' : undefined} onClick={() => setSelected(w.kind)}>
                {WORKFLOW_META[w.kind].name}
                <NavList.TrailingVisual>
                  <span className={`wf-state${w.enabled ? ' on' : ''}`}>{w.enabled ? 'On' : 'Off'}</span>
                </NavList.TrailingVisual>
              </NavList.Item>
            ))}
          </NavList.Group>
        </NavList>
      </aside>
      <main>
        <WorkflowEditor key={current.kind} project={project} workflow={current} statusField={q.data.status_field} />
      </main>
    </div>
  );
}

function WorkflowEditor({ project, workflow, statusField }: { project: ProjectFull; workflow: ProjectWorkflow; statusField: ProjectFull['fields'][number] | null }) {
  const qc = useQueryClient();
  const meta = WORKFLOW_META[workflow.kind];
  const [enabled, setEnabled] = useState(workflow.enabled);
  const [status, setStatus] = useState(workflow.config.status ?? '');
  const [filter, setFilter] = useState(workflow.config.filter ?? '');
  const [repo, setRepo] = useState(workflow.config.repo ?? project.repositories[0]?.full_name ?? '');
  useEffect(() => {
    setEnabled(workflow.enabled);
    setStatus(workflow.config.status ?? '');
    setFilter(workflow.config.filter ?? '');
    setRepo(workflow.config.repo ?? project.repositories[0]?.full_name ?? '');
  }, [workflow, project.repositories]);
  const save = useMutation({
    mutationFn: (v: { enabled: boolean }) => reef(`/projects/${project.owner.login}/${project.number}/workflows/${workflow.kind}`, { method: 'PUT', json: { enabled: v.enabled, config: meta.kind === 'status' || meta.kind === 'close' ? { status } : meta.kind === 'archive' ? { filter } : { repo, filter } } }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['project', project.owner.login.toLowerCase(), project.number] }),
  });
  const canEdit = project.permissions.admin;
  const dirty = enabled !== workflow.enabled || status !== (workflow.config.status ?? '') || filter !== (workflow.config.filter ?? '') || repo !== (workflow.config.repo ?? project.repositories[0]?.full_name ?? '');
  return (
    <div className="workflow-editor">
      <div className="workflow-head">
        <h2>{meta.name}</h2>
        <span style={{ flex: 1 }} />
        <ToggleSwitch
          checked={enabled}
          onClick={() => {
            if (!canEdit) return;
            setEnabled(!enabled);
            save.mutate({ enabled: !enabled });
          }}
          aria-labelledby={`wf-${workflow.kind}`}
          size="small"
          disabled={!canEdit}
        />
        <span id={`wf-${workflow.kind}`} className="sr-only" style={{ position: 'absolute', left: -9999 }}>
          {meta.name} workflow
        </span>
      </div>
      <div className="workflow-step">
        <span className="step-label">When</span>
        <div className="step-body">
          <p>{meta.when}</p>
          {meta.kind === 'archive' || meta.kind === 'add' ? (
            <FormControl>
              <FormControl.Label>Filter</FormControl.Label>
              <TextInput value={filter} onChange={(e) => setFilter(e.target.value)} block disabled={!canEdit} placeholder={meta.kind === 'archive' ? 'is:closed updated:<@today-2w' : 'is:issue,pr is:open label:bug'} />
              <FormControl.Caption>{meta.kind === 'archive' ? 'Supports is:, label:, status:, no:, assignee:, updated:, created: with @today-2w style dates.' : 'GitHub issue search qualifiers: is:, label:, assignee:, milestone:, and free text.'}</FormControl.Caption>
            </FormControl>
          ) : null}
          {meta.kind === 'add' ? (
            <FormControl>
              <FormControl.Label>Repository</FormControl.Label>
              <TextInput value={repo} onChange={(e) => setRepo(e.target.value)} placeholder="owner/repo" list={`wf-repos-${project.id}`} disabled={!canEdit} />
              <datalist id={`wf-repos-${project.id}`}>
                {project.repositories.map((r) => (
                  <option key={r.id} value={r.full_name} />
                ))}
              </datalist>
            </FormControl>
          ) : null}
          {meta.kind === 'close' ? <StatusSelect field={statusField} value={status} onChange={setStatus} disabled={!canEdit} /> : null}
        </div>
      </div>
      <div className="workflow-step">
        <span className="step-label">Then</span>
        <div className="step-body">
          <p>{meta.then}</p>
          {meta.kind === 'status' ? <StatusSelect field={statusField} value={status} onChange={setStatus} disabled={!canEdit} /> : null}
        </div>
      </div>
      {canEdit ? (
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <Button variant="primary" size="small" disabled={!dirty} loading={save.isPending} onClick={() => save.mutate({ enabled })}>
            Save and turn {enabled ? 'on' : 'off'}
          </Button>
          {save.isError ? <span className="small" style={{ color: 'var(--fgColor-danger)' }}>{(save.error as Error).message}</span> : null}
          {save.isSuccess && !dirty ? <span className="small muted">Saved</span> : null}
        </div>
      ) : (
        <p className="muted small">Only project admins can change workflows.</p>
      )}
    </div>
  );
}

function StatusSelect({ field, value, onChange, disabled }: { field: ProjectFull['fields'][number] | null; value: string; onChange: (v: string) => void; disabled: boolean }) {
  if (!field) return <p className="muted small">This project has no Status field.</p>;
  return (
    <FormControl>
      <FormControl.Label>Status</FormControl.Label>
      <Select value={value} onChange={(e) => onChange(e.target.value)} disabled={disabled}>
        <Select.Option value="">Choose a status…</Select.Option>
        {field.options.map((o) => (
          <Select.Option key={o.id} value={o.id}>
            {o.name}
          </Select.Option>
        ))}
      </Select>
      {value ? (
        <FormControl.Caption>
          <span className="dot" style={{ background: OPTION_DOTS[field.options.find((o) => o.id === value)?.color ?? 'GRAY'] }} /> {field.options.find((o) => o.id === value)?.name}
        </FormControl.Caption>
      ) : null}
    </FormControl>
  );
}
