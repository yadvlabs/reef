import { StarIcon } from '@primer/octicons-react';
import { Blankslate } from '@primer/react/experimental';
import { useQuery } from '@tanstack/react-query';
import { createFileRoute, Link } from '@tanstack/react-router';
import { AppHeader } from '../components/AppHeader';
import { Loading } from '../components/Loading';
import { api, type Repo } from '../lib/api';

export const Route = createFileRoute('/user/starred')({ component: Starred });

function Starred() {
  const q = useQuery({ queryKey: ['user', 'starred'], queryFn: () => api<Repo[]>('/user/starred?per_page=100') });
  return (
    <>
      <AppHeader />
      <div className="page narrow">
        <h1 style={{ fontSize: 24, fontWeight: 400, margin: '0 0 16px' }}>Your stars</h1>
        {q.isPending ? <Loading /> : null}
        <div className="repo-list">
          {q.data?.map((r) => (
            <div key={r.id} className="item">
              <div className="headline">
                <Link to="/$owner/$repo" params={{ owner: r.owner.login, repo: r.name }}>
                  {r.full_name}
                </Link>
                <span className="visibility-pill">{r.private ? 'Private' : 'Public'}</span>
              </div>
              {r.description ? <p className="desc">{r.description}</p> : null}
            </div>
          ))}
        </div>
        {q.data?.length === 0 ? (
          <Blankslate>
            <Blankslate.Visual>
              <StarIcon size="medium" />
            </Blankslate.Visual>
            <Blankslate.Heading>No starred repositories yet</Blankslate.Heading>
          </Blankslate>
        ) : null}
      </div>
    </>
  );
}
