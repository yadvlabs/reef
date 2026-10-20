import { Outlet, createRootRouteWithContext } from '@tanstack/react-router';
import type { QueryClient } from '@tanstack/react-query';
import { Crashed, NotFound } from '../components/Loading';

export const Route = createRootRouteWithContext<{ queryClient: QueryClient }>()({
  component: () => (
    <>
      <Outlet />
      <footer className="footer">
        <span>Reef and Crusty are &copy; {new Date().getFullYear()} Rohit Yadav and <a href="https://github.com/yadvlabs/reef" target="_blank" rel="noreferrer">YADV Labs</a> authors. MIT licensed.</span>
      </footer>
    </>
  ),
  notFoundComponent: () => <NotFound />,
  errorComponent: ({ error, reset }) => <Crashed error={error} reset={reset} />,
});
