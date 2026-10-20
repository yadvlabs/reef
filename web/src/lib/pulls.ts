import { queryOptions } from '@tanstack/react-query';
import { api, type Commit, type Repo, type User } from './api';
import { HTML, type Issue } from './issues';

export interface Pull extends Issue {
  head: { label: string; ref: string; sha: string; repo: Repo };
  base: { label: string; ref: string; sha: string; repo: Repo };
  draft: boolean;
  merged: boolean;
  merged_at: string | null;
  merged_by: User | null;
  merge_commit_sha: string | null;
  mergeable: boolean | null;
  mergeable_state: string;
  requested_reviewers: User[];
  commits: number;
  additions: number;
  deletions: number;
  changed_files: number;
  review_comments: number;
}

export interface PullFile {
  filename: string;
  status: string;
  additions: number;
  deletions: number;
  changes: number;
  patch: string | null;
  previous_filename: string | null;
}

export interface Review {
  id: number;
  user: User | null;
  body: string;
  body_html?: string;
  state: 'APPROVED' | 'CHANGES_REQUESTED' | 'COMMENTED' | 'DISMISSED' | 'PENDING';
  submitted_at: string | null;
  commit_id: string | null;
  author_association: string;
}

export interface ReviewComment {
  id: number;
  pull_request_review_id: number | null;
  diff_hunk: string;
  path: string;
  line: number | null;
  start_line: number | null;
  side: 'LEFT' | 'RIGHT';
  user: User | null;
  body: string;
  body_html: string;
  created_at: string;
  updated_at: string;
  in_reply_to_id: number | null;
  html_url: string;
  author_association: string;
  resolved: boolean;
  resolved_by: User | null;
  /** Belongs to the viewer's unsubmitted review. */
  pending: boolean;
}

export interface CheckRun {
  id: number;
  name: string;
  status: 'queued' | 'in_progress' | 'completed';
  conclusion: string | null;
  html_url: string;
  started_at: string | null;
  completed_at: string | null;
  app?: { name: string; slug: string };
}

export interface CommitStatus {
  state: 'success' | 'failure' | 'error' | 'pending';
  context: string;
  description: string | null;
  target_url: string | null;
  created_at: string;
}

export interface CombinedStatus {
  state: 'success' | 'failure' | 'pending';
  statuses: CommitStatus[];
  total_count: number;
}

/** A review thread: the root line comment and its replies, keyed for the diff view. */
export interface Thread {
  root: ReviewComment;
  replies: ReviewComment[];
}

export function threadsOf(comments: ReviewComment[]): Thread[] {
  const roots = comments.filter((c) => !c.in_reply_to_id);
  return roots.map((root) => ({ root, replies: comments.filter((c) => c.in_reply_to_id === root.id) }));
}

export const threadKey = (path: string, side: string, line: number | null) => `${path}\u0000${side}\u0000${line ?? ''}`;

export interface Compare {
  status: 'ahead' | 'behind' | 'identical' | 'diverged';
  ahead_by: number;
  behind_by: number;
  total_commits: number;
  commits: Commit[];
  files: PullFile[];
}

export const pullQuery = (full: string, number: number) => queryOptions({ queryKey: ['pulls', full.toLowerCase(), number], queryFn: () => api<Pull>(`/repos/${full}/pulls/${number}`, HTML) });
export const pullFilesQuery = (full: string, number: number) => queryOptions({ queryKey: ['pulls', full.toLowerCase(), number, 'files'], queryFn: () => api<PullFile[]>(`/repos/${full}/pulls/${number}/files?per_page=100`) });
export const pullCommitsQuery = (full: string, number: number) => queryOptions({ queryKey: ['pulls', full.toLowerCase(), number, 'commits'], queryFn: () => api<Commit[]>(`/repos/${full}/pulls/${number}/commits?per_page=100`) });
export const pullReviewsQuery = (full: string, number: number) => queryOptions({ queryKey: ['pulls', full.toLowerCase(), number, 'reviews'], queryFn: () => api<Review[]>(`/repos/${full}/pulls/${number}/reviews`, HTML) });
export const reviewCommentsQuery = (full: string, number: number) =>
  queryOptions({ queryKey: ['pulls', full.toLowerCase(), number, 'review-comments'], queryFn: () => api<ReviewComment[]>(`/repos/${full}/pulls/${number}/comments?per_page=100`) });
export const checkRunsQuery = (full: string, sha: string) =>
  queryOptions({ queryKey: ['checks', full.toLowerCase(), sha, 'runs'], queryFn: () => api<{ total_count: number; check_runs: CheckRun[] }>(`/repos/${full}/commits/${sha}/check-runs`), enabled: !!sha, refetchInterval: 30_000 });
export const combinedStatusQuery = (full: string, sha: string) =>
  queryOptions({ queryKey: ['checks', full.toLowerCase(), sha, 'status'], queryFn: () => api<CombinedStatus>(`/repos/${full}/commits/${sha}/status`), enabled: !!sha, refetchInterval: 30_000 });
export const compareQuery = (full: string, base: string, head: string) =>
  queryOptions({ queryKey: ['compare', full.toLowerCase(), base, head], queryFn: () => api<Compare>(`/repos/${full}/compare/${encodeURIComponent(base)}...${encodeURIComponent(head)}`), enabled: !!base && !!head && base !== head });
export const commitDetailQuery = (full: string, sha: string) => queryOptions({ queryKey: ['commit', full.toLowerCase(), sha, 'detail'], queryFn: () => api<Commit & { files: PullFile[]; stats: { additions: number; deletions: number } }>(`/repos/${full}/commits/${sha}`) });

// ----- unified diff parsing for the file viewer -----

export interface DiffLine {
  type: 'add' | 'del' | 'ctx' | 'hunk';
  old?: number;
  new?: number;
  text: string;
}

export function parsePatch(patch: string | null): DiffLine[] {
  if (!patch) return [];
  const out: DiffLine[] = [];
  let o = 0;
  let n = 0;
  for (const raw of patch.split('\n')) {
    if (raw.startsWith('@@')) {
      const m = raw.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      if (m) {
        o = Number(m[1]);
        n = Number(m[2]);
      }
      out.push({ type: 'hunk', text: raw });
    } else if (raw.startsWith('+')) {
      out.push({ type: 'add', new: n++, text: raw.slice(1) });
    } else if (raw.startsWith('-')) {
      out.push({ type: 'del', old: o++, text: raw.slice(1) });
    } else if (raw.startsWith('\\')) {
      out.push({ type: 'ctx', text: raw });
    } else {
      out.push({ type: 'ctx', old: o++, new: n++, text: raw.slice(1) });
    }
  }
  return out;
}

/** Rows for a side-by-side diff: deletions and additions in the same hunk are paired up. */
export interface SplitRow {
  left: DiffLine | null;
  right: DiffLine | null;
  hunk?: DiffLine;
}

export function splitRows(lines: DiffLine[]): SplitRow[] {
  const out: SplitRow[] = [];
  let i = 0;
  while (i < lines.length) {
    const l = lines[i];
    if (l.type === 'hunk') {
      out.push({ left: null, right: null, hunk: l });
      i++;
      continue;
    }
    if (l.type === 'ctx') {
      out.push({ left: l, right: l });
      i++;
      continue;
    }
    const dels: DiffLine[] = [];
    const adds: DiffLine[] = [];
    while (i < lines.length && lines[i].type === 'del') dels.push(lines[i++]);
    while (i < lines.length && lines[i].type === 'add') adds.push(lines[i++]);
    const n = Math.max(dels.length, adds.length);
    for (let k = 0; k < n; k++) out.push({ left: dels[k] ?? null, right: adds[k] ?? null });
    if (dels.length === 0 && adds.length === 0) i++;
  }
  return out;
}
