// Screenshot a page of a running Reef in headless Chrome, signed in through a session cookie,
// over the DevTools protocol (no browser automation package needed). Start Chrome first:
//   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --disable-gpu \
//     --hide-scrollbars --remote-debugging-port=9333 --user-data-dir=/tmp/chrome-cdp about:blank
// then: node scripts/screenshot.mjs reef_session <cookie value or -> out.png http://127.0.0.1:4920/rohit/test [height]
// The cookie value is the reef_session entry of a curl cookie jar after POST /api/reef/signup or /session.
const [cookieName, cookieValue, out, url, heightArg] = process.argv.slice(2);
const height = Number(heightArg || 1400);
const base = 'http://127.0.0.1:9333';
const t = await (await fetch(`${base}/json/new?about:blank`, { method: 'PUT' })).json();
const ws = new WebSocket(t.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0; const pending = new Map();
ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); } };
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
await send('Network.enable');
await send('Emulation.setDeviceMetricsOverride', { width: 1280, height, deviceScaleFactor: 1, mobile: false });
if (cookieValue && cookieValue !== '-') await send('Network.setCookie', { name: cookieName, value: cookieValue, url: new URL(url).origin + '/', path: '/' });
await send('Page.enable');
await send('Page.navigate', { url });
await new Promise((r) => setTimeout(r, 3500));
const shot = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
const { writeFileSync } = await import('node:fs');
writeFileSync(out, Buffer.from(shot.result.data, 'base64'));
await send('Target.closeTarget', { targetId: t.id }).catch(() => {});
ws.close();
console.log('wrote', out);
