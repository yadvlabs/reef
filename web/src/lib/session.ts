import { queryOptions, useQuery, useQueryClient } from '@tanstack/react-query';
import { reef, type Session } from './api';

export const sessionQuery = queryOptions({
  queryKey: ['session'],
  queryFn: () => reef<Session>('/session'),
  staleTime: 60_000,
});

export function useSession() {
  const q = useQuery(sessionQuery);
  return { session: q.data, user: q.data?.user ?? null, orgs: q.data?.orgs ?? [], instance: q.data?.instance, loading: q.isPending };
}

export function useRefreshSession() {
  const qc = useQueryClient();
  return () => qc.invalidateQueries({ queryKey: ['session'] });
}
