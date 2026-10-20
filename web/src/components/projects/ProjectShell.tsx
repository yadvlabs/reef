// The project header shared by the board page and its sub-pages (insights, workflows,
// archive, status updates): title, latest status, and the actions on the right.
import { ActionList, ActionMenu, IconButton } from '@primer/react';
import { ArchiveIcon, GearIcon, GraphIcon, KebabHorizontalIcon, WorkflowIcon, HistoryIcon } from '@primer/octicons-react';
import { Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import type { ProjectFull } from '../../lib/projects';
import { AppHeader } from '../AppHeader';
import { StatusPill, StatusUpdateDialog } from './StatusUpdates';

export function ProjectHeader({ project, onSettings }: { project: ProjectFull; onSettings?: () => void }) {
  const p = project;
  const navigate = useNavigate();
  const [adding, setAdding] = useState(false);
  const params = { owner: p.owner.login, number: String(p.number) };
  const goSettings = () => (onSettings ? onSettings() : navigate({ to: '/$owner/projects/$number', params, search: { settings: true } }));
  return (
    <div className="project-head">
      <Link to="/$owner/projects/$number" params={params} search={{}} className="project-title">
        <h1>{p.title}</h1>
      </Link>
      {p.short_description ? <span className="muted small project-desc">{p.short_description}</span> : null}
      {p.closed ? <span className="visibility-pill">Closed</span> : null}
      {!p.public ? <span className="visibility-pill">Private</span> : null}
      <span style={{ flex: 1 }} />
      {p.permissions.write ? (
        <button className="status-button" onClick={() => setAdding(true)} title={p.status_update ? 'Add status update' : 'Add the first status update'}>
          {p.status_update ? <StatusPill update={p.status_update} /> : <span className="add-status">Add status update</span>}
        </button>
      ) : (
        <Link to="/$owner/projects/$number/status" params={params} className="status-button">
          <StatusPill update={p.status_update} />
        </Link>
      )}
      <Link to="/$owner/projects/$number/insights" params={params} aria-label="Insights" title="Insights" className="icon-link">
        <GraphIcon />
      </Link>
      {p.permissions.admin ? <IconButton icon={GearIcon} aria-label="Settings" variant="invisible" size="small" onClick={goSettings} /> : null}
      <ActionMenu>
        <ActionMenu.Anchor>
          <IconButton icon={KebabHorizontalIcon} aria-label="More project options" variant="invisible" size="small" />
        </ActionMenu.Anchor>
        <ActionMenu.Overlay align="end">
          <ActionList>
            <ActionList.LinkItem href={`/${p.owner.login}/projects/${p.number}/status`}>
              <ActionList.LeadingVisual>
                <HistoryIcon />
              </ActionList.LeadingVisual>
              Status updates
            </ActionList.LinkItem>
            <ActionList.LinkItem href={`/${p.owner.login}/projects/${p.number}/workflows`}>
              <ActionList.LeadingVisual>
                <WorkflowIcon />
              </ActionList.LeadingVisual>
              Workflows
            </ActionList.LinkItem>
            <ActionList.LinkItem href={`/${p.owner.login}/projects/${p.number}/archive`}>
              <ActionList.LeadingVisual>
                <ArchiveIcon />
              </ActionList.LeadingVisual>
              Archived items
              {p.archived_count ? <ActionList.TrailingVisual>{p.archived_count}</ActionList.TrailingVisual> : null}
            </ActionList.LinkItem>
            {p.permissions.admin ? (
              <>
                <ActionList.Divider />
                <ActionList.Item onSelect={goSettings}>
                  <ActionList.LeadingVisual>
                    <GearIcon />
                  </ActionList.LeadingVisual>
                  Settings
                </ActionList.Item>
              </>
            ) : null}
          </ActionList>
        </ActionMenu.Overlay>
      </ActionMenu>
      {adding ? <StatusUpdateDialog project={p} onClose={() => setAdding(false)} /> : null}
    </div>
  );
}

/// Page frame for the sub-pages: global header, project header, then content.
export function ProjectShell({ project, children }: { project: ProjectFull; children: React.ReactNode }) {
  return (
    <>
      <AppHeader context={{ owner: project.owner.login, section: { label: 'Projects', to: 'projects' } }} />
      <ProjectHeader project={project} />
      {children}
    </>
  );
}
