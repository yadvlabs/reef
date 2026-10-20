import { Button, Flash, Spinner } from '@primer/react';
import { Crusty } from './Crusty';
import { ApiError } from '../lib/api';

export function Loading() {
  return (
    <div style={{ display: 'flex', justifyContent: 'center', padding: 48 }}>
      <Spinner />
    </div>
  );
}

export function ErrorFlash({ error }: { error: unknown }) {
  const msg = error instanceof ApiError ? [error.message, ...error.fieldErrors].join(': ') : error instanceof Error ? error.message : String(error);
  return <Flash variant="danger">{msg}</Flash>;
}

export function NotFound({ what = 'page' }: { what?: string }) {
  return (
    <div className="page crusty-page">
      <Crusty mood="peekaboo" size={200} />
      <h1>404</h1>
      <p className="muted">Crusty looked everywhere: this {what} could not be found, or you do not have access to it.</p>
      <div className="actions">
        <Button as="a" href="/">Go home</Button>
      </div>
    </div>
  );
}

/** What a page shows when rendering it failed (the router's error boundary). */
export function Crashed({ error, reset }: { error: unknown; reset?: () => void }) {
  const msg = error instanceof ApiError ? error.message : error instanceof Error ? error.message : String(error);
  return (
    <div className="page crusty-page">
      <Crusty mood="resting" size={200} />
      <h1>Something went wrong</h1>
      <p className="muted">Crusty is taking a moment. Reload the page; if it keeps happening, tell your administrator.</p>
      <p className="muted small" style={{ marginTop: 8 }}>
        <code>{msg}</code>
      </p>
      <div className="actions">
        <Button onClick={() => (reset ? reset() : window.location.reload())}>Try again</Button>
        <Button as="a" href="/" variant="invisible">Go home</Button>
      </div>
    </div>
  );
}
