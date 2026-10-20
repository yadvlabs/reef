import { createFileRoute, Outlet } from '@tanstack/react-router';

export const Route = createFileRoute('/$owner/$repo/actions')({ component: () => <Outlet /> });
