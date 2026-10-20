import { createFileRoute, Outlet } from '@tanstack/react-router';

export const Route = createFileRoute('/$owner/projects')({ component: () => <Outlet /> });
