import { IconButton } from '@primer/react';
import { BellIcon } from '@primer/octicons-react';
import { useNavigate } from '@tanstack/react-router';
import { useSession } from '../lib/session';

export function NotificationsBell() {
  const { session } = useSession();
  const navigate = useNavigate();
  const unread = session?.unread_notifications ?? 0;
  return (
    <span className="bell">
      <IconButton icon={BellIcon} aria-label={unread ? `${unread} unread notifications` : 'Notifications'} variant="default" onClick={() => navigate({ to: '/notifications', search: {} })} />
      {unread ? <span className="bell-dot" /> : null}
    </span>
  );
}
