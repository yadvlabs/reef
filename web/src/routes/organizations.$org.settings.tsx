import { ActionList, ActionMenu, Avatar, Button, FormControl, TextInput, Textarea } from '@primer/react';
import { PersonIcon, PeopleIcon, PinIcon, WebhookIcon, AlertIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { createFileRoute, Link, useNavigate } from '@tanstack/react-router';
import { useState } from 'react';
import { AppHeader } from '../components/AppHeader';
import { WebhooksPanel } from '../components/webhooks/WebhooksPanel';
import { AvatarUpload } from '../components/AvatarUpload';
import { ErrorFlash, Loading, NotFound } from '../components/Loading';
import { accountQuery, orgMembersQuery, OwnerTabs } from '../components/OwnerTabs';
import { PinsEditor } from '../components/Pins';
import { api, ApiError, reef, type User } from '../lib/api';
import { useSession } from '../lib/session';

export const Route = createFileRoute('/organizations/$org/settings')({
  validateSearch: (s: Record<string, unknown>): { tab?: string } => (typeof s.tab === 'string' ? { tab: s.tab } : {}),
  component: OrgSettings,
});

function OrgSettings() {
  const { org } = Route.useParams();
  const { tab = 'profile' } = Route.useSearch();
  const { user, loading } = useSession();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const account = useQuery(accountQuery(org));
  const members = useQuery({ queryKey: ['org', org.toLowerCase(), 'memberships'], queryFn: () => api<User[]>(`/orgs/${org}/members`) });
  const admins = useQuery({ queryKey: ['org', org.toLowerCase(), 'admins'], queryFn: () => api<User[]>(`/orgs/${org}/members?role=admin`) });
  const [error, setError] = useState<unknown>(null);
  const [saved, setSaved] = useState(false);
  const [newMember, setNewMember] = useState('');
  const [newRole, setNewRole] = useState('member');
  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ['account', org.toLowerCase()] });
    qc.invalidateQueries({ queryKey: ['org', org.toLowerCase()] });
  };
  const save = useMutation({ mutationFn: (body: Record<string, unknown>) => api(`/orgs/${org}`, { method: 'PATCH', json: body }), onSuccess: () => { invalidate(); setSaved(true); }, onError: setError });
  const addMember = useMutation({ mutationFn: ({ login, role }: { login: string; role: string }) => api(`/orgs/${org}/memberships/${login}`, { method: 'PUT', json: { role } }), onSuccess: () => { invalidate(); setNewMember(''); }, onError: setError });
  const removeMember = useMutation({ mutationFn: (login: string) => api(`/orgs/${org}/memberships/${login}`, { method: 'DELETE' }), onSuccess: invalidate, onError: setError });
  if (loading || account.isPending) return <><AppHeader context={{ owner: org }} /><Loading /></>;
  if (!user) {
    navigate({ to: '/login', search: { return_to: location.pathname } });
    return null;
  }
  if (account.isError) return <><AppHeader context={{ owner: org }} />{account.error instanceof ApiError && account.error.status === 404 ? <NotFound what="organization" /> : <ErrorFlash error={account.error} />}</>;
  const a = account.data;
  const isAdmin = user.site_admin || (admins.data ?? []).some((m) => m.login === user.login);
  return (
    <>
      <AppHeader context={{ owner: a.login }} />
      <OwnerTabs account={a} current="overview" />
      <div className="page narrow">
        <div className="settings-layout">
          <nav className="settings-nav">
            <Link to="/organizations/$org/settings" params={{ org }} search={{ tab: 'profile' }} className={tab === 'profile' ? 'active' : ''}>
              <PersonIcon /> Profile
            </Link>
            <Link to="/organizations/$org/settings" params={{ org }} search={{ tab: 'members' }} className={tab === 'members' ? 'active' : ''}>
              <PeopleIcon /> Members
            </Link>
            <Link to="/organizations/$org/settings" params={{ org }} search={{ tab: 'pins' }} className={tab === 'pins' ? 'active' : ''}>
              <PinIcon /> Pinned repositories
            </Link>
            <Link to="/organizations/$org/settings" params={{ org }} search={{ tab: 'hooks' }} className={tab === 'hooks' ? 'active' : ''}>
              <WebhookIcon /> Webhooks
            </Link>
            <Link to="/organizations/$org/settings" params={{ org }} search={{ tab: 'danger' }} className={tab === 'danger' ? 'active' : ''}>
              <AlertIcon /> Rename or delete
            </Link>
          </nav>
          <div>
            {!isAdmin ? <ErrorFlash error="You must be an organization owner to change these settings." /> : null}
            {error ? <ErrorFlash error={error} /> : null}
            {tab === 'hooks' ? (
              <WebhooksPanel base={`/orgs/${a.login}`} canAdmin={isAdmin} />
            ) : tab === 'profile' ? (
              <>
                <h2 className="section-title">Organization profile</h2>
                <div className="profile-form-grid">
                  <form
                    className="form-stack"
                    onSubmit={(e) => {
                      e.preventDefault();
                      setSaved(false);
                      const f = new FormData(e.currentTarget);
                      save.mutate({ name: f.get('name'), description: f.get('description'), blog: f.get('blog'), location: f.get('location'), company: f.get('company'), billing_email: f.get('billing_email') });
                    }}
                  >
                    <FormControl>
                      <FormControl.Label>Display name</FormControl.Label>
                      <TextInput name="name" defaultValue={a.name ?? ''} block disabled={!isAdmin} />
                    </FormControl>
                    <FormControl>
                      <FormControl.Label>Description</FormControl.Label>
                      <Textarea name="description" defaultValue={a.description ?? ''} block rows={3} disabled={!isAdmin} />
                    </FormControl>
                    <FormControl>
                      <FormControl.Label>Website</FormControl.Label>
                      <TextInput name="blog" defaultValue={a.blog ?? ''} block disabled={!isAdmin} />
                    </FormControl>
                    <FormControl>
                      <FormControl.Label>Location</FormControl.Label>
                      <TextInput name="location" defaultValue={a.location ?? ''} block disabled={!isAdmin} />
                    </FormControl>
                    <FormControl>
                      <FormControl.Label>Company</FormControl.Label>
                      <TextInput name="company" defaultValue={a.company ?? ''} block disabled={!isAdmin} />
                    </FormControl>
                    <FormControl>
                      <FormControl.Label>Billing email</FormControl.Label>
                      <TextInput name="billing_email" type="email" defaultValue={''} block disabled={!isAdmin} />
                    </FormControl>
                    <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
                      <Button type="submit" variant="primary" disabled={!isAdmin} loading={save.isPending}>
                        Update profile
                      </Button>
                      {saved ? <span className="muted small">Saved.</span> : null}
                    </div>
                  </form>
                  <div>
                    <h3 style={{ fontSize: 14, margin: '0 0 8px' }}>Organization picture</h3>
                    <AvatarUpload avatarUrl={a.avatar_url} endpoint={`/orgs/${a.login}/avatar`} square onChanged={invalidate} />
                  </div>
                </div>
              </>
            ) : tab === 'danger' ? (
              <DangerZone org={a.login} isAdmin={isAdmin} />
            ) : tab === 'members' ? (
              <>
                <h2 className="section-title">Members</h2>
                <form
                  style={{ display: 'flex', gap: 8, marginBottom: 16 }}
                  onSubmit={(e) => {
                    e.preventDefault();
                    if (newMember.trim()) addMember.mutate({ login: newMember.trim(), role: newRole });
                  }}
                >
                  <TextInput value={newMember} onChange={(e) => setNewMember(e.target.value)} placeholder="Username" disabled={!isAdmin} />
                  <select className="select" value={newRole} onChange={(e) => setNewRole(e.target.value)} disabled={!isAdmin}>
                    <option value="member">Member</option>
                    <option value="admin">Owner</option>
                  </select>
                  <Button type="submit" variant="primary" disabled={!isAdmin || !newMember.trim()} loading={addMember.isPending}>
                    Add member
                  </Button>
                </form>
                <div className="key-list">
                  {members.data?.map((m) => {
                    const isOwner = (admins.data ?? []).some((x) => x.id === m.id);
                    return (
                      <div key={m.id} className="item" style={{ alignItems: 'center' }}>
                        <Avatar src={m.avatar_url} size={32} alt="" />
                        <div className="body">
                          <Link to="/$owner" params={{ owner: m.login }}>
                            <strong>{m.login}</strong>
                          </Link>
                          <div className="muted small">{m.name}</div>
                        </div>
                        {isAdmin ? (
                          <ActionMenu>
                            <ActionMenu.Button size="small">{isOwner ? 'Owner' : 'Member'}</ActionMenu.Button>
                            <ActionMenu.Overlay align="end">
                              <ActionList selectionVariant="single">
                                <ActionList.Item selected={isOwner} onSelect={() => addMember.mutate({ login: m.login, role: 'admin' })}>
                                  Owner
                                  <ActionList.Description variant="block">Full access to the organization, its repositories and settings.</ActionList.Description>
                                </ActionList.Item>
                                <ActionList.Item selected={!isOwner} onSelect={() => addMember.mutate({ login: m.login, role: 'member' })}>
                                  Member
                                  <ActionList.Description variant="block">Can create repositories and see other members.</ActionList.Description>
                                </ActionList.Item>
                                <ActionList.Divider />
                                <ActionList.Item variant="danger" onSelect={() => confirm(`Remove ${m.login} from ${org}?`) && removeMember.mutate(m.login)}>
                                  Remove from organization
                                </ActionList.Item>
                              </ActionList>
                            </ActionMenu.Overlay>
                          </ActionMenu>
                        ) : (
                          <span className="visibility-pill">{isOwner ? 'Owner' : 'Member'}</span>
                        )}
                      </div>
                    );
                  })}
                </div>
              </>
            ) : (
              <>
                <h2 className="section-title">Pinned repositories</h2>
                <p className="muted small">Up to six repositories shown at the top of the organization profile.</p>
                <PinsEditor owner={a.login} isOrg canEdit={isAdmin} />
              </>
            )}
          </div>
        </div>
      </div>
    </>
  );
}

export { reef as _reef, orgMembersQuery as _omq };

/** Rename the organization or delete it, like GitHub's Danger zone. */
function DangerZone({ org, isAdmin }: { org: string; isAdmin: boolean }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [login, setLogin] = useState(org);
  const [confirm, setConfirm] = useState('');
  const [error, setError] = useState<unknown>(null);
  const rename = useMutation({
    mutationFn: () => reef<User>(`/orgs/${org}/login`, { method: 'PATCH', json: { login: login.trim() } }),
    onSuccess: (o) => {
      qc.invalidateQueries({ queryKey: ['account'] });
      qc.invalidateQueries({ queryKey: ['session'] });
      navigate({ to: '/organizations/$org/settings', params: { org: o.login }, search: { tab: 'danger' } });
    },
    onError: setError,
  });
  const remove = useMutation({
    mutationFn: () => api(`/orgs/${org}`, { method: 'DELETE' }),
    onSuccess: () => {
      qc.invalidateQueries();
      navigate({ to: '/' });
    },
    onError: setError,
  });
  return (
    <div className="form-stack">
      <h2 className="section-title">Rename organization</h2>
      {error ? <ErrorFlash error={error} /> : null}
      <p className="muted small" style={{ margin: 0 }}>
        Renaming changes the organization's URL and every clone URL under it. Links to the old name are not redirected.
      </p>
      <FormControl>
        <FormControl.Label>Organization name</FormControl.Label>
        <TextInput value={login} onChange={(e) => setLogin(e.target.value)} block disabled={!isAdmin} />
      </FormControl>
      <div>
        <Button onClick={() => rename.mutate()} disabled={!isAdmin || !login.trim() || login.trim() === org} loading={rename.isPending}>
          Rename organization
        </Button>
      </div>
      <div className="danger-zone">
        <h2 className="section-title" style={{ color: 'var(--fgColor-danger)' }}>Danger zone</h2>
        <div className="danger-row">
          <div>
            <strong>Delete this organization</strong>
            <div className="muted small">Once you delete an organization, its repositories, projects, webhooks and memberships are gone. There is no going back.</div>
          </div>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
            <TextInput value={confirm} onChange={(e) => setConfirm(e.target.value)} placeholder={`Type ${org} to confirm`} size="small" disabled={!isAdmin} aria-label="Confirm organization name" />
            <Button variant="danger" size="small" disabled={!isAdmin || confirm !== org} loading={remove.isPending} onClick={() => remove.mutate()}>
              Delete organization
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
