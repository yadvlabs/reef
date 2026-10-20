import { queryOptions } from '@tanstack/react-query';
import { api, type User } from './api';

export interface Workflow {
  id: number;
  name: string;
  path: string;
  state: string;
  html_url: string;
}
export interface WorkflowRun {
  id: number;
  name: string;
  display_title: string;
  head_branch: string | null;
  head_sha: string;
  path: string;
  run_number: number;
  run_attempt: number;
  event: string;
  status: 'queued' | 'in_progress' | 'completed';
  conclusion: string | null;
  workflow_id: number;
  created_at: string;
  updated_at: string;
  run_started_at: string | null;
  actor: User | null;
  triggering_actor: User | null;
  head_commit: { id: string; message: string } | null;
  error?: string | null;
}
export interface Step {
  name: string;
  status: 'queued' | 'in_progress' | 'completed';
  conclusion: string | null;
  number: number;
  started_at: string | null;
  completed_at: string | null;
}
export interface Job {
  id: number;
  run_id: number;
  name: string;
  status: 'waiting' | 'queued' | 'in_progress' | 'completed';
  conclusion: string | null;
  started_at: string | null;
  completed_at: string | null;
  steps: Step[];
  labels: string[];
  runner_name: string | null;
  job_key: string;
}
export interface Runner {
  id: number;
  name: string;
  os: string;
  status: 'online' | 'offline';
  busy: boolean;
  ephemeral: boolean;
  labels: { id: number; name: string; type: string }[];
  version: string | null;
  last_seen_at: string | null;
}

export const workflowsQuery = (full: string) => queryOptions({ queryKey: ['actions', full.toLowerCase(), 'workflows'], queryFn: () => api<{ workflows: Workflow[] }>(`/repos/${full}/actions/workflows`) });
export const runsQuery = (full: string, workflow?: number, branch?: string) =>
  queryOptions({
    queryKey: ['actions', full.toLowerCase(), 'runs', workflow ?? 0, branch ?? ''],
    queryFn: () => api<{ total_count: number; workflow_runs: WorkflowRun[] }>(workflow ? `/repos/${full}/actions/workflows/${workflow}/runs?per_page=50${branch ? `&branch=${encodeURIComponent(branch)}` : ''}` : `/repos/${full}/actions/runs?per_page=50${branch ? `&branch=${encodeURIComponent(branch)}` : ''}`),
    refetchInterval: 5000,
  });
export const runQuery = (full: string, id: number) => queryOptions({ queryKey: ['actions', full.toLowerCase(), 'run', id], queryFn: () => api<WorkflowRun>(`/repos/${full}/actions/runs/${id}`), refetchInterval: (q) => (q.state.data?.status === 'completed' ? false : 3000) });
export const runJobsQuery = (full: string, id: number) => queryOptions({ queryKey: ['actions', full.toLowerCase(), 'run', id, 'jobs'], queryFn: () => api<{ jobs: Job[] }>(`/repos/${full}/actions/runs/${id}/jobs`), refetchInterval: 3000 });
export const jobLogQuery = (full: string, id: number, live: boolean) => queryOptions({ queryKey: ['actions', full.toLowerCase(), 'job', id, 'log'], queryFn: () => api<string>(`/repos/${full}/actions/jobs/${id}/logs`, { raw: true }), refetchInterval: live ? 2000 : false });
export const runnersQuery = (full: string) => queryOptions({ queryKey: ['actions', full.toLowerCase(), 'runners'], queryFn: () => api<{ runners: Runner[] }>(`/repos/${full}/actions/runners`), refetchInterval: 10000 });

export function runDuration(r: { run_started_at?: string | null; started_at?: string | null; created_at?: string; updated_at?: string; completed_at?: string | null; status: string }): string {
  const start = new Date(r.run_started_at ?? r.started_at ?? r.created_at ?? Date.now()).getTime();
  const end = r.status === 'completed' ? new Date(r.completed_at ?? r.updated_at ?? Date.now()).getTime() : Date.now();
  const s = Math.max(0, Math.round((end - start) / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/// Split a job log into the step groups the runner emits (`##[group]` … `##[endgroup]`).
export function splitLog(text: string): { name: string; lines: string[] }[] {
  const out: { name: string; lines: string[] }[] = [];
  let cur: { name: string; lines: string[] } | null = null;
  for (const raw of text.split('\n')) {
    const line = raw.replace(/^\d{4}-\d{2}-\d{2}T[\d:.]+Z /, '');
    if (line.startsWith('##[group]')) {
      cur = { name: line.slice(9), lines: [] };
      out.push(cur);
    } else if (line.startsWith('##[endgroup]')) {
      cur = null;
    } else if (cur) {
      cur.lines.push(line);
    } else if (line.trim()) {
      cur = { name: 'Output', lines: [line] };
      out.push(cur);
    }
  }
  return out;
}
