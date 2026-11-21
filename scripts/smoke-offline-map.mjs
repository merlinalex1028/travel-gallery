/** Production frontend only: isolated Chrome, browser demo data, no native profile. */
import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtemp, mkdir, readFile, realpath, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { extname, join, resolve, sep } from 'node:path';
import desktop from '../src-tauri/tauri.conf.json' with { type: 'json' };

const root = process.cwd();
const frontend = await realpath(join(root, 'dist'));
const output = join(root, 'artifacts/offline-map-headless');
const chrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const budget = ['scripts/check-test-artifact-budget.py', '--extra-root', frontend];
function check(args = []) {
  const result = spawnSync('python3', [...budget, ...args], { encoding: 'utf8' });
  if (result.status) throw new Error(result.stdout || result.stderr);
  return JSON.parse(result.stdout);
}
check(['--planned-files', '650', '--planned-bytes', String(300 * 1024 ** 2)]);
await mkdir(output);
const run = await mkdtemp(join(tmpdir(), 'travel-gallery-map-browser-'));
const requests = [],
  exceptions = [];
let browser,
  socket,
  stderr = '',
  session,
  next = 0,
  interrupted = false;
const pending = new Map();
function interrupt() {
  interrupted = true;
  for (const request of pending.values()) {
    clearTimeout(request.timer);
    request.fail(new Error('Browser map check interrupted'));
  }
  pending.clear();
  browser?.kill('SIGTERM');
}
for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.on(signal, interrupt);
const delay = (ms) => new Promise((done) => setTimeout(done, ms));
const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    const path = await realpath(resolve(frontend, `.${pathname === '/' ? '/index.html' : pathname}`));
    if (!path.startsWith(frontend + sep) || !(await stat(path)).isFile()) throw new Error('not found');
    const types = {
      '.html': 'text/html',
      '.js': 'text/javascript',
      '.css': 'text/css',
      '.ttf': 'font/ttf',
      '.txt': 'text/plain',
      '.svg': 'image/svg+xml',
    };
    response.writeHead(200, {
      'Content-Type': types[extname(path)] ?? 'application/octet-stream',
      'Content-Security-Policy': desktop.app.security.csp,
      'Cache-Control': 'no-store',
    });
    response.end(await readFile(path));
  } catch {
    response.writeHead(404).end();
  }
});
await new Promise((done) => server.listen(0, '127.0.0.1', done));
const origin = `http://127.0.0.1:${server.address().port}`;
function command(method, params = {}, target = session) {
  if (interrupted) throw new Error('Browser map check interrupted');
  const id = ++next;
  return new Promise((done, fail) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      fail(new Error(`CDP timeout: ${method}`));
    }, 20000);
    pending.set(id, { done, fail, timer });
    socket.send(JSON.stringify({ id, method, params, ...(target ? { sessionId: target } : {}) }));
  });
}
async function evaluate(expression) {
  const response = await command('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (response.exceptionDetails) throw new Error(JSON.stringify(response.exceptionDetails));
  return response.result.value;
}
try {
  browser = spawn(
    chrome,
    [
      '--headless=new',
      `--user-data-dir=${join(run, 'profile')}`,
      `--disk-cache-dir=${join(run, 'cache')}`,
      '--remote-debugging-port=0',
      '--window-size=1440,1000',
      '--disable-background-networking',
      '--disable-component-update',
      '--disable-default-apps',
      '--disable-sync',
      '--disable-extensions',
      '--no-first-run',
      '--no-default-browser-check',
      '--password-store=basic',
      '--use-angle=swiftshader',
      '--enable-unsafe-swiftshader',
      'about:blank',
    ],
    { detached: true, env: { ...process.env, TMPDIR: run }, stdio: ['ignore', 'ignore', 'pipe'] },
  );
  browser.stderr.on('data', (bytes) => {
    stderr = (stderr + bytes).slice(-3000);
  });
  let active;
  for (let attempt = 0; attempt < 200; attempt++) {
    if (interrupted) throw new Error('Browser map check interrupted');
    try {
      active = (await readFile(join(run, 'profile/DevToolsActivePort'), 'utf8')).trim().split('\n');
      break;
    } catch {
      await delay(100);
    }
  }
  if (!active) throw new Error(`Chrome did not start: ${stderr}`);
  socket = new WebSocket(`ws://127.0.0.1:${active[0]}${active[1]}`);
  await new Promise((done, fail) => {
    socket.addEventListener('open', done, { once: true });
    socket.addEventListener('error', fail, { once: true });
  });
  socket.addEventListener('message', ({ data }) => {
    const event = JSON.parse(data);
    if (event.id) {
      const request = pending.get(event.id);
      if (!request) return;
      clearTimeout(request.timer);
      pending.delete(event.id);
      if (event.error) request.fail(new Error(JSON.stringify(event.error)));
      else request.done(event.result);
    } else if (event.method === 'Network.requestWillBeSent') requests.push(event.params.request.url);
    else if (event.method === 'Runtime.exceptionThrown') exceptions.push(event.params.exceptionDetails.text);
  });
  const { targetId } = await command('Target.createTarget', { url: 'about:blank' }, null);
  ({ sessionId: session } = await command('Target.attachToTarget', { targetId, flatten: true }, null));
  await command('Network.enable');
  await command('Runtime.enable');
  await command('Page.enable');
  await command('Page.navigate', { url: `${origin}/#/map` });
  let state;
  for (let attempt = 0; attempt < 150; attempt++) {
    state = await evaluate(`({
      markers: document.querySelectorAll('.map-marker').length,
      errors: [...document.querySelectorAll('.map-page [role="alert"]')].map(e => e.textContent),
      fontLoaded: [...document.fonts].some(f => f.family === 'Travel Map Noto Sans' && f.status === 'loaded'),
      fontFamily: document.querySelector('.map-marker') ? getComputedStyle(document.querySelector('.map-marker')).fontFamily : '',
      canvas: !!document.querySelector('.maplibregl-canvas')
    })`);
    if (
      state.markers &&
      state.fontLoaded &&
      state.canvas &&
      !state.errors.length &&
      state.fontFamily.includes('Travel Map Noto Sans')
    )
      break;
    await delay(100);
  }
  if (
    !state.markers ||
    !state.fontLoaded ||
    !state.canvas ||
    state.errors.length ||
    !state.fontFamily.includes('Travel Map Noto Sans')
  )
    throw new Error(`Map failed to render: ${JSON.stringify(state)}`);
  await evaluate(`document.querySelector('.maplibregl-ctrl-zoom-out').click()`);
  await delay(400);
  // Keep the world's land/coast visible in the screenshot instead of only the demo's inland extent.
  for (let count = 0; count < 5; count++) {
    await evaluate(`document.querySelector('.maplibregl-ctrl-zoom-out').click()`);
    await delay(350);
  }
  await delay(1000);
  const afterZoomErrors = await evaluate(
    `[...document.querySelectorAll('.map-page [role="alert"]')].map(e => e.textContent)`,
  );
  if (afterZoomErrors.length) throw new Error(JSON.stringify(afterZoomErrors));
  const external = requests.filter((url) => !url.startsWith(origin + '/') && !/^(data|blob):/.test(url));
  if (external.length || exceptions.length) throw new Error(JSON.stringify({ external, exceptions }));
  const image = await command('Page.captureScreenshot', { format: 'png' });
  await writeFile(join(output, 'map.png'), Buffer.from(image.data, 'base64'));
  const result = {
    passed: true,
    scope: 'Production frontend in isolated headless Chrome with browser demo data',
    ...state,
    requestCount: requests.length,
    externalRequests: external,
    fontRequests: requests.filter((url) => url.endsWith('.ttf')).length,
    chrome: (await command('Browser.getVersion', {}, null)).product,
    budget: check(['--extra-root', run]),
  };
  await writeFile(join(output, 'result.json'), JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify(result));
} finally {
  if (socket?.readyState === WebSocket.OPEN) {
    try {
      await command('Browser.close', {}, null);
    } catch {}
    socket.close();
  }
  if (browser) {
    for (let attempt = 0; browser.exitCode === null && attempt < 50; attempt++) await delay(100);
    if (browser.exitCode === null) {
      process.kill(-browser.pid, 'SIGTERM');
      await new Promise((done) => browser.once('exit', done));
    }
  }
  await new Promise((done) => server.close(done));
  await rm(run, { recursive: true, force: true });
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) process.off(signal, interrupt);
  check();
}
