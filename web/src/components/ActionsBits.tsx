import { CheckCircleFillIcon, XCircleFillIcon, DotFillIcon, SkipFillIcon, StopIcon, ClockIcon } from '@primer/octicons-react';

/// Status icon for runs, jobs and steps, matching GitHub's colours.
export function StatusIcon({ status, conclusion, size = 16 }: { status: string; conclusion: string | null; size?: number }) {
  if (status !== 'completed') {
    if (status === 'in_progress') return <span className="spin" style={{ width: size, height: size }} aria-label="In progress" />;
    return <ClockIcon size={size} fill="var(--fgColor-attention)" aria-label="Queued" />;
  }
  switch (conclusion) {
    case 'success':
      return <CheckCircleFillIcon size={size} fill="var(--fgColor-success)" aria-label="Success" />;
    case 'failure':
    case 'timed_out':
    case 'startup_failure':
      return <XCircleFillIcon size={size} fill="var(--fgColor-danger)" aria-label="Failure" />;
    case 'cancelled':
      return <StopIcon size={size} fill="var(--fgColor-muted)" aria-label="Cancelled" />;
    case 'skipped':
      return <SkipFillIcon size={size} fill="var(--fgColor-muted)" aria-label="Skipped" />;
    default:
      return <DotFillIcon size={size} fill="var(--fgColor-muted)" />;
  }
}

export function statusLabel(status: string, conclusion: string | null): string {
  if (status !== 'completed') return status === 'in_progress' ? 'In progress' : status === 'waiting' ? 'Waiting' : 'Queued';
  return (conclusion ?? 'unknown').replace('_', ' ').replace(/^\w/, (c) => c.toUpperCase());
}
