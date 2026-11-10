// Explicit loopback control in a separately built QA application. Never loaded
// by a normal production build, and never sends a request without a trusted click.
export function installOfflineNetworkCanary(record: (event: Record<string, unknown>) => void) {
  const nonce = crypto.randomUUID();
  const endpoint = `http://127.0.0.1:39841/travel-gallery-canary/${nonce}`;
  const panel = document.createElement('section');
  panel.setAttribute('aria-label', '独立离线网络控制');
  Object.assign(panel.style, {
    position: 'fixed',
    bottom: '16px',
    right: '16px',
    zIndex: '5000',
    padding: '16px',
    maxWidth: '540px',
    background: '#fff',
    color: '#14251c',
    border: '2px solid #315d49',
    borderRadius: '8px',
    font: '14px system-ui',
    boxShadow: '0 4px 24px #0004',
  });
  const title = document.createElement('strong');
  title.textContent = '验收专用 · 回环网络控制';
  const description = document.createElement('p');
  description.textContent =
    '手动发送一次本机请求，结果与独立监听日志共同核对。此控制不等于完整离线验收通过。';
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = '发送回环控制请求';
  const status = document.createElement('p');
  status.setAttribute('role', 'status');
  status.textContent = '尚未发送';
  panel.append(title, description, button, status);
  document.body.append(panel);
  record({ type: 'offlineCanaryInstalled', endpoint, nonce, scope: 'explicit QA loopback control' });
  document.addEventListener('securitypolicyviolation', (event) => {
    if (event.blockedURI === endpoint || event.blockedURI.startsWith('http://127.0.0.1:39841'))
      record({
        type: 'offlineCanaryCspViolation',
        nonce,
        blockedURI: event.blockedURI,
        directive: event.effectiveDirective,
        policy: event.originalPolicy,
      });
  });
  button.addEventListener('click', async (event) => {
    if (!event.isTrusted || button.disabled) return;
    button.disabled = true;
    status.textContent = '正在发送回环请求…';
    record({ type: 'offlineCanaryAttempt', endpoint, nonce, trusted: event.isTrusted });
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 4000);
    try {
      const response = await fetch(endpoint, {
        method: 'GET',
        credentials: 'omit',
        cache: 'no-store',
        redirect: 'error',
        referrerPolicy: 'no-referrer',
        signal: controller.signal,
      });
      const payload = await response.text();
      const matched = response.status === 200 && payload === `travel-gallery-canary-ok:${nonce}`;
      status.textContent = matched ? '回环请求成功：需要核对网络隔离范围' : '回环响应不匹配：控制无效';
      record({ type: 'offlineCanaryResponse', nonce, status: response.status, matched });
    } catch (error) {
      status.textContent = '回环请求失败：需要结合 CSP、监听器及系统记录判断原因';
      record({
        type: 'offlineCanaryError',
        nonce,
        name: error instanceof Error ? error.name : '',
        message: String(error),
      });
    } finally {
      clearTimeout(timeout);
    }
  });
}
