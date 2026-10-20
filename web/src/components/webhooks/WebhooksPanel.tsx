// Webhooks settings: list, add/edit form and recent deliveries, for a repository
// (`/repos/{o}/{r}`) or an organization (`/orgs/{org}`), styled after GitHub's pages.
import { ActionList, ActionMenu, Button, Checkbox, FormControl, Radio, RadioGroup, RelativeTime, Select, TextInput } from '@primer/react';
import { Blankslate } from '@primer/react/experimental';
import { AlertIcon, CheckCircleFillIcon, CheckIcon, ChevronDownIcon, ChevronRightIcon, DotFillIcon, KebabHorizontalIcon, SyncIcon, TrashIcon, WebhookIcon, XCircleFillIcon } from '@primer/octicons-react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { api } from '../../lib/api';
import { deliveriesQuery, deliveryQuery, HOOK_EVENTS, hooksQuery, type DeliverySummary, type Hook, type HookInput } from '../../lib/webhooks';
import { ErrorFlash, Loading } from '../Loading';

type Mode = { kind: 'list' } | { kind: 'new' } | { kind: 'edit'; id: number };

export function WebhooksPanel({ base, canAdmin }: { base: string; canAdmin: boolean }) {
  const [mode, setMode] = useState<Mode>({ kind: 'list' });
  const hooks = useQuery({ ...hooksQuery(base), enabled: canAdmin });
  if (!canAdmin) return <ErrorFlash error="You need admin access to manage webhooks." />;
  if (hooks.isPending) return <Loading />;
  if (hooks.isError) return <ErrorFlash error={hooks.error} />;
  if (mode.kind === 'new') return <HookEditor base={base} onDone={() => setMode({ kind: 'list' })} />;
  if (mode.kind === 'edit') {
    const hook = hooks.data.find((h) => h.id === mode.id);
    if (!hook) return <ErrorFlash error="This webhook no longer exists." />;
    return <HookEditor base={base} hook={hook} onDone={() => setMode({ kind: 'list' })} />;
  }
  return <HookList base={base} hooks={hooks.data} onNew={() => setMode({ kind: 'new' })} onEdit={(id) => setMode({ kind: 'edit', id })} />;
}

function StatusDot({ hook }: { hook: Hook }) {
  const s = hook.last_response.status;
  const color = s === 'active' ? 'var(--fgColor-success)' : s === 'misconfigured' ? 'var(--fgColor-danger)' : 'var(--fgColor-muted)';
  const title = s === 'active' ? `Last delivery was successful (${hook.last_response.code})` : s === 'misconfigured' ? `Last delivery failed (${hook.last_response.code ?? 'no response'})` : 'No deliveries yet';
  return <DotFillIcon fill={color} aria-label={title} />;
}

function eventsSummary(events: string[]): string {
  if (events.includes('*')) return 'Send me everything';
  if (events.length === 1 && events[0] === 'push') return 'Just the push event';
  return `${events.length} events: ${events.slice(0, 4).join(', ')}${events.length > 4 ? '…' : ''}`;
}

function HookList({ base, hooks, onNew, onEdit }: { base: string; hooks: Hook[]; onNew: () => void; onEdit: (id: number) => void }) {
  const qc = useQueryClient();
  const invalidate = () => qc.invalidateQueries({ queryKey: ['hooks', base] });
  const remove = useMutation({ mutationFn: (id: number) => api(`${base}/hooks/${id}`, { method: 'DELETE' }), onSuccess: invalidate });
  const ping = useMutation({ mutationFn: (id: number) => api(`${base}/hooks/${id}/pings`, { method: 'POST' }), onSuccess: invalidate });
  return (
    <div className="hooks-panel">
      <div className="hooks-head">
        <h2 className="section-title" style={{ margin: 0 }}>Webhooks</h2>
        <Button variant="primary" size="small" onClick={onNew}>
          Add webhook
        </Button>
      </div>
      <p className="muted">Webhooks allow external services to be notified when certain events happen. When the specified events happen, we'll send a POST request to each of the URLs you provide.</p>
      {remove.isError ? <ErrorFlash error={remove.error} /> : null}
      {hooks.length === 0 ? (
        <Blankslate border>
          <Blankslate.Visual>
            <WebhookIcon size="medium" />
          </Blankslate.Visual>
          <Blankslate.Heading>There are no webhooks for this {base.startsWith('/orgs') ? 'organization' : 'repository'}</Blankslate.Heading>
          <Blankslate.Description>Webhooks let you build or set up integrations which subscribe to certain events.</Blankslate.Description>
        </Blankslate>
      ) : (
        <div className="key-list">
          {hooks.map((h) => (
            <div key={h.id} className="item hook-row">
              <StatusDot hook={h} />
              <div className="body">
                <button className="linkish hook-url" onClick={() => onEdit(h.id)}>
                  {h.config.url}
                </button>
                <div className="muted small">
                  {h.active ? eventsSummary(h.events) : 'Disabled'} {h.config.content_type === 'form' ? '· form' : ''}
                </div>
              </div>
              <Button size="small" onClick={() => onEdit(h.id)}>
                Edit
              </Button>
              <ActionMenu>
                <ActionMenu.Anchor>
                  <button className="linkish muted" aria-label="Webhook options">
                    <KebabHorizontalIcon />
                  </button>
                </ActionMenu.Anchor>
                <ActionMenu.Overlay align="end">
                  <ActionList>
                    <ActionList.Item onSelect={() => ping.mutate(h.id)}>
                      <ActionList.LeadingVisual>
                        <SyncIcon />
                      </ActionList.LeadingVisual>
                      Ping
                    </ActionList.Item>
                    <ActionList.Item variant="danger" onSelect={() => confirm('Delete this webhook? Deliveries will stop immediately.') && remove.mutate(h.id)}>
                      <ActionList.LeadingVisual>
                        <TrashIcon />
                      </ActionList.LeadingVisual>
                      Delete
                    </ActionList.Item>
                  </ActionList>
                </ActionMenu.Overlay>
              </ActionMenu>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

type Choice = 'push' | 'all' | 'select';

function HookEditor({ base, hook, onDone }: { base: string; hook?: Hook; onDone: () => void }) {
  const qc = useQueryClient();
  const [url, setUrl] = useState(hook?.config.url ?? '');
  const [contentType, setContentType] = useState<'json' | 'form'>(hook?.config.content_type ?? 'json');
  const [secret, setSecret] = useState(hook?.config.secret ? '********' : '');
  const [insecure, setInsecure] = useState(hook?.config.insecure_ssl === '1');
  const [active, setActive] = useState(hook?.active ?? true);
  const initialChoice: Choice = !hook ? 'push' : hook.events.includes('*') ? 'all' : hook.events.length === 1 && hook.events[0] === 'push' ? 'push' : 'select';
  const [choice, setChoice] = useState<Choice>(initialChoice);
  const [selected, setSelected] = useState<string[]>(hook?.events.filter((e) => e !== '*') ?? ['push']);
  const events = choice === 'all' ? ['*'] : choice === 'push' ? ['push'] : selected;
  const invalidate = () => qc.invalidateQueries({ queryKey: ['hooks', base] });
  const save = useMutation({
    mutationFn: (input: HookInput) => (hook ? api<Hook>(`${base}/hooks/${hook.id}`, { method: 'PATCH', json: input }) : api<Hook>(`${base}/hooks`, { method: 'POST', json: { name: 'web', ...input } })),
    onSuccess: () => {
      invalidate();
      onDone();
    },
  });
  const ping = useMutation({ mutationFn: () => api(`${base}/hooks/${hook!.id}/pings`, { method: 'POST' }), onSuccess: () => qc.invalidateQueries({ queryKey: ['hooks', base, hook!.id, 'deliveries'] }) });
  const remove = useMutation({ mutationFn: () => api(`${base}/hooks/${hook!.id}`, { method: 'DELETE' }), onSuccess: () => { invalidate(); onDone(); } });
  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const config: HookInput['config'] = { url: url.trim(), content_type: contentType, insecure_ssl: insecure ? '1' : '0' };
    if (secret !== '********') config.secret = secret;
    save.mutate({ active, events: events.length ? events : ['push'], config });
  };
  return (
    <div className="hooks-panel">
      <div className="hooks-head">
        <h2 className="section-title" style={{ margin: 0 }}>
          <button className="linkish" onClick={onDone}>
            Webhooks
          </button>{' '}
          / {hook ? 'Manage webhook' : 'Add webhook'}
        </h2>
        {hook ? (
          <div style={{ display: 'flex', gap: 8 }}>
            <Button size="small" leadingVisual={SyncIcon} onClick={() => ping.mutate()} disabled={ping.isPending}>
              Ping
            </Button>
            <Button size="small" variant="danger" leadingVisual={TrashIcon} onClick={() => confirm('Delete this webhook?') && remove.mutate()}>
              Delete webhook
            </Button>
          </div>
        ) : null}
      </div>
      <p className="muted">We'll send a <code>POST</code> request to the URL below with details of any subscribed events. You can also specify which data format you'd like to receive (JSON, <code>x-www-form-urlencoded</code>, etc).</p>
      {save.isError ? <ErrorFlash error={save.error} /> : null}
      <form className="form-stack hook-form" onSubmit={submit}>
        <FormControl required>
          <FormControl.Label>Payload URL</FormControl.Label>
          <TextInput block value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://example.com/postreceive" type="url" />
        </FormControl>
        <FormControl>
          <FormControl.Label>Content type</FormControl.Label>
          <Select value={contentType} onChange={(e) => setContentType(e.target.value as 'json' | 'form')}>
            <Select.Option value="json">application/json</Select.Option>
            <Select.Option value="form">application/x-www-form-urlencoded</Select.Option>
          </Select>
        </FormControl>
        <FormControl>
          <FormControl.Label>Secret</FormControl.Label>
          <TextInput block value={secret} onChange={(e) => setSecret(e.target.value)} type="password" autoComplete="new-password" placeholder={hook?.config.secret ? 'Leave as is to keep the current secret' : ''} />
          <FormControl.Caption>
            Used to compute the <code>X-Hub-Signature-256</code> header so the receiver can verify the delivery came from here.
          </FormControl.Caption>
        </FormControl>
        {url.startsWith('https://') ? (
          <fieldset className="hook-fieldset">
            <legend>SSL verification</legend>
            <p className="muted small">By default, we verify SSL certificates when delivering payloads.</p>
            <RadioGroup name="ssl" onChange={(v) => setInsecure(v === 'disable')}>
              <FormControl>
                <Radio value="enable" defaultChecked={!insecure} />
                <FormControl.Label>Enable SSL verification</FormControl.Label>
              </FormControl>
              <FormControl>
                <Radio value="disable" defaultChecked={insecure} />
                <FormControl.Label>
                  <AlertIcon fill="var(--fgColor-attention)" /> Disable (not recommended)
                </FormControl.Label>
              </FormControl>
            </RadioGroup>
          </fieldset>
        ) : null}
        <fieldset className="hook-fieldset">
          <legend>Which events would you like to trigger this webhook?</legend>
          <RadioGroup name="events-choice" onChange={(v) => setChoice(v as Choice)}>
            <FormControl>
              <Radio value="push" defaultChecked={choice === 'push'} />
              <FormControl.Label>Just the <code>push</code> event.</FormControl.Label>
            </FormControl>
            <FormControl>
              <Radio value="all" defaultChecked={choice === 'all'} />
              <FormControl.Label>Send me everything.</FormControl.Label>
            </FormControl>
            <FormControl>
              <Radio value="select" defaultChecked={choice === 'select'} />
              <FormControl.Label>Let me select individual events.</FormControl.Label>
            </FormControl>
          </RadioGroup>
          {choice === 'select' ? (
            <div className="hook-events-grid">
              {HOOK_EVENTS.map((ev) => (
                <FormControl key={ev.name}>
                  <Checkbox checked={selected.includes(ev.name)} onChange={(e) => setSelected(e.target.checked ? [...selected, ev.name] : selected.filter((x) => x !== ev.name))} />
                  <FormControl.Label>{ev.label}</FormControl.Label>
                  <FormControl.Caption>{ev.help}</FormControl.Caption>
                </FormControl>
              ))}
            </div>
          ) : null}
        </fieldset>
        <FormControl>
          <Checkbox checked={active} onChange={(e) => setActive(e.target.checked)} />
          <FormControl.Label>Active</FormControl.Label>
          <FormControl.Caption>We will deliver event details when this hook is triggered.</FormControl.Caption>
        </FormControl>
        <div style={{ display: 'flex', gap: 8 }}>
          <Button type="submit" variant="primary" disabled={save.isPending || !url.trim()}>
            {hook ? 'Update webhook' : 'Add webhook'}
          </Button>
          <Button type="button" onClick={onDone}>
            Cancel
          </Button>
        </div>
      </form>
      {hook ? <Deliveries base={base} hook={hook} /> : null}
    </div>
  );
}

function DeliveryIcon({ d }: { d: DeliverySummary }) {
  if (d.status === 'OK') return <CheckCircleFillIcon fill="var(--fgColor-success)" />;
  if (d.status === 'Pending' || d.status === 'Retrying') return <DotFillIcon fill="var(--fgColor-attention)" />;
  return <XCircleFillIcon fill="var(--fgColor-danger)" />;
}

function Deliveries({ base, hook }: { base: string; hook: Hook }) {
  const q = useQuery(deliveriesQuery(base, hook.id));
  const [open, setOpen] = useState<number | null>(null);
  return (
    <section className="hook-deliveries">
      <h3>Recent Deliveries</h3>
      {q.isPending ? <Loading /> : null}
      {q.isError ? <ErrorFlash error={q.error} /> : null}
      {q.data && q.data.length === 0 ? <p className="muted">No deliveries yet. Trigger a subscribed event, or use Ping.</p> : null}
      {q.data?.length ? (
        <div className="key-list">
          {q.data.map((d) => (
            <div key={d.id} className="item hook-delivery">
              <button className="linkish hook-delivery-row" onClick={() => setOpen(open === d.id ? null : d.id)} aria-expanded={open === d.id}>
                {open === d.id ? <ChevronDownIcon /> : <ChevronRightIcon />}
                <DeliveryIcon d={d} />
                <code className="hook-guid">{d.guid}</code>
                <span className="muted small">
                  {d.event}
                  {d.action ? `.${d.action}` : ''}
                </span>
                {d.redelivery ? <span className="counter">redelivery</span> : null}
                <span style={{ flex: 1 }} />
                <span className="muted small">{d.status_code ? `${d.status_code}` : d.status}</span>
                <span className="muted small">
                  <RelativeTime datetime={d.delivered_at} />
                </span>
              </button>
              {open === d.id ? <DeliveryDetail base={base} hook={hook} id={d.id} /> : null}
            </div>
          ))}
        </div>
      ) : null}
    </section>
  );
}

function DeliveryDetail({ base, hook, id }: { base: string; hook: Hook; id: number }) {
  const qc = useQueryClient();
  const q = useQuery(deliveryQuery(base, hook.id, id));
  const [tab, setTab] = useState<'request' | 'response'>('request');
  const redeliver = useMutation({
    mutationFn: () => api(`${base}/hooks/${hook.id}/deliveries/${id}/attempts`, { method: 'POST' }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['hooks', base, hook.id, 'deliveries'] }),
  });
  if (q.isPending) return <Loading />;
  if (q.isError) return <ErrorFlash error={q.error} />;
  const d = q.data;
  const fmt = (v: unknown) => (typeof v === 'string' ? v : JSON.stringify(v, null, 2));
  return (
    <div className="hook-delivery-detail">
      <div className="release-tabs" style={{ margin: 0 }}>
        <button type="button" className={`tab${tab === 'request' ? ' active' : ''}`} onClick={() => setTab('request')}>
          Request
        </button>
        <button type="button" className={`tab${tab === 'response' ? ' active' : ''}`} onClick={() => setTab('response')}>
          Response {d.status_code ? <span className="counter">{d.status_code}</span> : null}
        </button>
        <span style={{ flex: 1 }} />
        <span className="muted small" style={{ alignSelf: 'center' }}>
          {d.duration != null ? `Completed in ${d.duration.toFixed(2)} seconds` : ''} · attempt {d.attempt}
        </span>
        <Button size="small" leadingVisual={redeliver.isSuccess ? CheckIcon : SyncIcon} onClick={() => redeliver.mutate()} disabled={redeliver.isPending} style={{ marginLeft: 8 }}>
          Redeliver
        </Button>
      </div>
      {tab === 'request' ? (
        <>
          <h4>Headers</h4>
          <pre className="hook-pre">{Object.entries(d.request.headers).map(([k, v]) => `${k}: ${v}`).join('\n')}</pre>
          <h4>Payload</h4>
          <pre className="hook-pre">{fmt(d.request.payload)}</pre>
        </>
      ) : (
        <>
          <h4>Headers</h4>
          <pre className="hook-pre">{Object.entries(d.response.headers ?? {}).map(([k, v]) => `${k}: ${v}`).join('\n') || '(no response)'}</pre>
          <h4>Body</h4>
          <pre className="hook-pre">{d.response.payload ?? ''}</pre>
        </>
      )}
    </div>
  );
}
