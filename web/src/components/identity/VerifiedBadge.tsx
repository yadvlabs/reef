// GitHub's commit signature badge: "Verified" (green) for a good signature, "Unverified"
// (grey) for a signature Reef cannot vouch for. Unsigned commits render nothing.
import { ActionMenu } from '@primer/react';
import { CheckIcon, ShieldLockIcon, XIcon } from '@primer/octicons-react';
import type { Commit } from '../../lib/api';

const REASONS: Record<string, string> = {
  valid: "This commit was signed with the committer's verified signature.",
  unknown_key: 'This commit was signed with a key that is not registered on any account.',
  bad_email: 'The email in this signature does not match the committer email or the key does not belong to the committer.',
  unverified_email: 'The committer email address is not verified.',
  expired_key: 'The key that signed this commit has expired.',
  invalid: 'The signature in this commit could not be verified.',
  unknown_signature_type: 'This commit was signed with an unsupported signature format.',
};

function keyLine(v: NonNullable<Commit['commit']['verification']>): string | null {
  const sig = v.signature ?? '';
  if (sig.includes('BEGIN SSH SIGNATURE')) return 'SSH signature';
  if (sig.includes('BEGIN PGP SIGNATURE')) return 'GPG signature';
  return null;
}

export function VerifiedBadge({ commit, size = 'small' }: { commit: Commit; size?: 'small' | 'medium' }) {
  const v = commit.commit.verification;
  if (!v || v.reason === 'unsigned' || (!v.signature && !v.verified)) return null;
  const login = commit.committer?.login ?? commit.commit.committer.name;
  const text = REASONS[v.reason] ?? v.reason;
  return (
    <ActionMenu>
      <ActionMenu.Anchor>
        <button type="button" className={`sig-badge ${v.verified ? 'verified' : 'unverified'} ${size}`} aria-label={v.verified ? 'Verified signature' : 'Unverified signature'} onClick={(e) => e.stopPropagation()}>
          {v.verified ? 'Verified' : 'Unverified'}
        </button>
      </ActionMenu.Anchor>
      <ActionMenu.Overlay width="medium" align="start">
        <div className="sig-popover">
          <div className="sig-popover-head">
            <span className={`sig-icon ${v.verified ? 'verified' : 'unverified'}`}>{v.verified ? <CheckIcon /> : <XIcon />}</span>
            <strong>{v.verified ? 'This commit was signed with the committer’s verified signature.' : 'This commit is signed, but the signature could not be verified.'}</strong>
          </div>
          {!v.verified ? <p className="small">{text}</p> : null}
          <div className="sig-meta small muted">
            <ShieldLockIcon /> <strong>{login}</strong> {keyLine(v) ?? 'Signature'}
            {v.verified ? ' · verified' : ` · ${v.reason.replace(/_/g, ' ')}`}
          </div>
        </div>
      </ActionMenu.Overlay>
    </ActionMenu>
  );
}
