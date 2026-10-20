import '@primer/primitives/dist/css/primitives.css';
import '@primer/primitives/dist/css/functional/themes/light.css';
import '@primer/primitives/dist/css/functional/themes/dark.css';
import './styles.css';
import './styles/projects.css';
import './styles/code.css';
import './styles/pulls.css';
import './styles/security.css';
import './styles/identity.css';
import './styles/webhooks.css';
import './styles/mirror.css';

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { BaseStyles, ThemeProvider } from '@primer/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider, createRouter } from '@tanstack/react-router';
import { routeTree } from './routeTree.gen';

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false, staleTime: 10_000 } },
});

const router = createRouter({
  routeTree,
  context: { queryClient },
  defaultPreload: 'intent',
  scrollRestoration: true,
});

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider colorMode="auto">
      <BaseStyles>
        <QueryClientProvider client={queryClient}>
          <RouterProvider router={router} />
        </QueryClientProvider>
      </BaseStyles>
    </ThemeProvider>
  </StrictMode>,
);
