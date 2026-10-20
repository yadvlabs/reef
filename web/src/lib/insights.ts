// Queries for repository insights (GitHub `stats/*` endpoints), pulse, per-entry last commits
// and user contribution calendars.
import { queryOptions } from '@tanstack/react-query';
import { api, reef, type Contributor, type User } from './api';

export interface ContributorStats {
  author: Contributor;
  total: number;
  weeks: { w: number; a: number; d: number; c: number }[];
}
export interface WeekActivity {
  days: number[];
  total: number;
  week: number;
}
export type CodeFrequency = [number, number, number][];

export interface PulseItem {
  number: number;
  title: string;
  html_url: string;
  user: User;
  created_at: string;
  closed_at: string | null;
  merged_at: string | null;
}
export interface Pulse {
  since: string;
  until: string;
  commits: { count: number; authors: number; additions: number; deletions: number; files_changed: number };
  authors: { author: Contributor; commits: number }[];
  merged_pulls: PulseItem[];
  open_pulls: PulseItem[];
  closed_issues: PulseItem[];
  new_issues: PulseItem[];
  counts: { merged_pulls: number; open_pulls: number; closed_issues: number; new_issues: number; active_pulls: number; active_issues: number };
}

export interface TreeCommit {
  sha: string;
  message: string;
  date: string;
  author: User | null;
  author_name: string;
}
export interface TreeCommits {
  commits: number;
  entries: Record<string, TreeCommit>;
}

export interface ContributionDay {
  date: string;
  count: number;
  commits: number;
  issues: number;
  pulls: number;
}
export interface ContributionMonth {
  month: string;
  commits: { repo: string; count: number }[];
  issues: { repo: string; count: number }[];
  pulls: { repo: string; count: number }[];
}
export interface Contributions {
  from: string;
  to: string;
  total: number;
  days: ContributionDay[];
  months: ContributionMonth[];
}

const key = (owner: string, repo: string, ...rest: unknown[]) => ['repo', owner.toLowerCase(), repo.toLowerCase(), ...rest];

export const contributorStatsQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: key(owner, repo, 'stats', 'contributors'), queryFn: () => api<ContributorStats[]>(`/repos/${owner}/${repo}/stats/contributors`), staleTime: 120_000 });
export const commitActivityQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: key(owner, repo, 'stats', 'commit_activity'), queryFn: () => api<WeekActivity[]>(`/repos/${owner}/${repo}/stats/commit_activity`), staleTime: 120_000 });
export const codeFrequencyQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: key(owner, repo, 'stats', 'code_frequency'), queryFn: () => api<CodeFrequency>(`/repos/${owner}/${repo}/stats/code_frequency`), staleTime: 120_000 });
export const punchCardQuery = (owner: string, repo: string) =>
  queryOptions({ queryKey: key(owner, repo, 'stats', 'punch_card'), queryFn: () => api<[number, number, number][]>(`/repos/${owner}/${repo}/stats/punch_card`), staleTime: 120_000 });
export const pulseQuery = (owner: string, repo: string, days: number) =>
  queryOptions({ queryKey: key(owner, repo, 'pulse', days), queryFn: () => reef<Pulse>(`/repos/${owner}/${repo}/pulse?days=${days}`), staleTime: 60_000 });
export const treeCommitsQuery = (owner: string, repo: string, rev: string, path: string) =>
  queryOptions({
    queryKey: key(owner, repo, 'tree-commits', rev, path),
    queryFn: () => reef<TreeCommits>(`/repos/${owner}/${repo}/tree-commits?rev=${encodeURIComponent(rev)}&path=${encodeURIComponent(path)}`),
    staleTime: 60_000,
  });
export const contributionsQuery = (login: string, from?: string, to?: string) =>
  queryOptions({
    queryKey: ['account', login.toLowerCase(), 'contributions', from ?? '', to ?? ''],
    queryFn: () => reef<Contributions>(`/users/${login}/contributions${from ? `?from=${from}&to=${to ?? ''}` : ''}`),
    staleTime: 120_000,
  });

/** Sunday-start week timestamp (seconds) `months` months before now, for period filters. */
export function sinceWeeks(months: number | null): number {
  if (months === null) return 0;
  const d = new Date();
  d.setUTCMonth(d.getUTCMonth() - months);
  return Math.floor(d.getTime() / 1000);
}

export function fmtInt(n: number): string {
  return n.toLocaleString('en-US');
}
