// Webhooks (GitHub REST shapes) for repositories and organizations.
import { queryOptions } from '@tanstack/react-query';
import { api } from './api';

export interface HookConfig {
  url: string;
  content_type: 'json' | 'form';
  insecure_ssl: '0' | '1';
  secret?: string;
}
export interface Hook {
  id: number;
  type: 'Repository' | 'Organization';
  name: 'web';
  active: boolean;
  events: string[];
  config: HookConfig;
  created_at: string;
  updated_at: string;
  url: string;
  last_response: { code: number | null; status: 'active' | 'misconfigured' | 'unused'; message: string };
}
export interface DeliverySummary {
  id: number;
  guid: string;
  delivered_at: string;
  redelivery: boolean;
  duration: number | null;
  status: string;
  status_code: number | null;
  event: string;
  action: string | null;
  attempt: number;
}
export interface DeliveryFull extends DeliverySummary {
  request: { headers: Record<string, string>; payload: unknown };
  response: { headers: Record<string, string>; payload: string | null };
}
export interface HookInput {
  active: boolean;
  events: string[];
  config: { url: string; content_type: 'json' | 'form'; secret?: string; insecure_ssl: '0' | '1' };
}

/** Events a hook can subscribe to, with GitHub's labels and blurbs. */
export const HOOK_EVENTS: { name: string; label: string; help: string }[] = [
  { name: 'push', label: 'Pushes', help: 'Git push to a repository.' },
  { name: 'create', label: 'Branch or tag creation', help: 'Branch or tag created.' },
  { name: 'delete', label: 'Branch or tag deletion', help: 'Branch or tag deleted.' },
  { name: 'issues', label: 'Issues', help: 'Issue opened, edited, closed, reopened, assigned, labeled, milestoned, locked.' },
  { name: 'issue_comment', label: 'Issue comments', help: 'Comment created on an issue or pull request.' },
  { name: 'pull_request', label: 'Pull requests', help: 'Pull request opened, closed, reopened, synchronized, ready for review, review requested.' },
  { name: 'pull_request_review', label: 'Pull request reviews', help: 'Pull request review submitted.' },
  { name: 'pull_request_review_comment', label: 'Pull request review comments', help: 'Comment on a pull request diff.' },
  { name: 'release', label: 'Releases', help: 'Release published.' },
  { name: 'star', label: 'Stars', help: 'A star is created.' },
  { name: 'watch', label: 'Watches', help: 'User stars a repository.' },
  { name: 'fork', label: 'Forks', help: 'Repository forked.' },
  { name: 'gollum', label: 'Wiki', help: 'Wiki page updated.' },
  { name: 'repository', label: 'Repositories', help: 'Repository created.' },
  { name: 'workflow_run', label: 'Workflow runs', help: 'Workflow run completed.' },
  { name: 'workflow_job', label: 'Workflow jobs', help: 'Workflow job completed.' },
];

export const hooksQuery = (base: string) => queryOptions({ queryKey: ['hooks', base], queryFn: () => api<Hook[]>(`${base}/hooks?per_page=100`) });
export const deliveriesQuery = (base: string, id: number) => queryOptions({ queryKey: ['hooks', base, id, 'deliveries'], queryFn: () => api<DeliverySummary[]>(`${base}/hooks/${id}/deliveries?per_page=50`), refetchInterval: 15_000 });
export const deliveryQuery = (base: string, id: number, delivery: number) => queryOptions({ queryKey: ['hooks', base, id, 'deliveries', delivery], queryFn: () => api<DeliveryFull>(`${base}/hooks/${id}/deliveries/${delivery}`) });
