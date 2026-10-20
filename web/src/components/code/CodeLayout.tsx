// The code view frame: an optional "Files" tree panel on the left (like GitHub's), the
// content in the middle, and, at the repository root, the About sidebar on the right.
import { IconButton } from '@primer/react';
import { SidebarCollapseIcon, SidebarExpandIcon } from '@primer/octicons-react';
import { createContext, useContext, useEffect, useState } from 'react';
import type { Repo } from '../../lib/api';
import { BranchMenu } from '../RepoCode';
import { FileTreePanel } from './FileTreePanel';

const STORAGE_KEY = 'reef.code.panel';

/** Rendered by toolbars inside the layout: the "expand panel" button when the panel is closed. */
export const PanelToggle = createContext<React.ReactNode>(null);

export function usePanelToggle() {
  return useContext(PanelToggle);
}

export function CodeLayout({ repo, rev, path, children, aside }: { repo: Repo; rev: string; path: string; children: React.ReactNode; aside?: React.ReactNode }) {
  const [open, setOpen] = useState<boolean>(() => {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === 'open') return true;
    if (stored === 'closed') return false;
    return path !== '';
  });
  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, open ? 'open' : 'closed');
  }, [open]);
  const toggle = !open ? <IconButton icon={SidebarExpandIcon} aria-label="Expand file tree" size="small" variant="invisible" onClick={() => setOpen(true)} className="panel-toggle" /> : null;
  return (
    <div className={`code-view${open ? ' panel-open' : ''}${aside ? ' with-aside' : ''}`}>
      {open ? (
        <aside className="file-panel" aria-label="Files">
          <div className="file-panel-head">
            <span className="file-panel-title">Files</span>
            <span style={{ flex: 1 }} />
            <IconButton icon={SidebarCollapseIcon} aria-label="Collapse file tree" size="small" variant="invisible" onClick={() => setOpen(false)} />
          </div>
          <div className="file-panel-branch">
            <BranchMenu repo={repo} rev={rev} path={path} mode="tree" />
          </div>
          <FileTreePanel repo={repo} rev={rev} current={path} />
        </aside>
      ) : null}
      <PanelToggle.Provider value={toggle}>
        <div className="code-main">{children}</div>
      </PanelToggle.Provider>
      {aside ? <div className="code-aside">{aside}</div> : null}
    </div>
  );
}
