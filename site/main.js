const root = document.documentElement;
const themeButton = document.querySelector('.theme-toggle');
const systemTheme = matchMedia('(prefers-color-scheme: dark)');
let preferredTheme;
try {
  preferredTheme = localStorage.getItem('travel-gallery-site-theme');
} catch {
  // The site remains usable when browser storage is disabled.
}
function setTheme(theme) {
  root.dataset.theme = theme;
  themeButton.setAttribute('aria-pressed', String(theme === 'dark'));
  themeButton.setAttribute('aria-label', theme === 'dark' ? '切换为浅色主题' : '切换为深色主题');
}
setTheme(
  preferredTheme === 'dark' || preferredTheme === 'light'
    ? preferredTheme
    : systemTheme.matches
      ? 'dark'
      : 'light',
);
themeButton.addEventListener('click', () => {
  preferredTheme = root.dataset.theme === 'dark' ? 'light' : 'dark';
  setTheme(preferredTheme);
  try {
    localStorage.setItem('travel-gallery-site-theme', preferredTheme);
  } catch {
    // Theme switching also works for this visit without storage.
  }
});
systemTheme.addEventListener('change', () => {
  if (!preferredTheme) setTheme(systemTheme.matches ? 'dark' : 'light');
});

const reviewButtons = [...document.querySelectorAll('[data-review]')];
const memory = document.querySelector('.memory-card');
const reviewStatus = document.querySelector('.review-badge');
const labels = {
  keep: '已保留 · 这个瞬间值得',
  reject: '已标记淘汰 · 还可以再想想',
  favorite: '已收藏 · 留住这份心动',
};
function review(action) {
  const next = memory.dataset.state === action ? 'unreviewed' : action;
  memory.dataset.state = next;
  reviewStatus.textContent = labels[next] ?? '待你发现';
  for (const button of reviewButtons)
    button.setAttribute('aria-pressed', String(button.dataset.review === next));
}
for (const button of reviewButtons) {
  button.addEventListener('click', () => review(button.dataset.review));
}
document.querySelector('.review-controls').addEventListener('keydown', (event) => {
  if (event.ctrlKey || event.metaKey || event.altKey || event.repeat) return;
  const action = { p: 'keep', x: 'reject', f: 'favorite' }[event.key.toLowerCase()];
  if (action) {
    event.preventDefault();
    review(action);
  }
});

// Download metadata is generated during deployment; no visitor-side GitHub API call is needed.
try {
  const response = await fetch(new URL('./downloads.json', import.meta.url));
  if (!response.ok) throw new Error('Download metadata unavailable');
  const release = await response.json();
  if (release.tag) {
    for (const status of document.querySelectorAll('[data-release-status]'))
      status.textContent = `${release.tag} · ${release.prerelease ? '预发布体验版' : '正式发布'}`;
    for (const platform of ['macos', 'windows']) {
      const file = release.assets[platform];
      for (const button of document.querySelectorAll(`[data-download="${platform}"]`)) {
        if (file) {
          button.href = file.url;
          button.setAttribute(
            'aria-label',
            `下载 ${release.tag} ${platform === 'macos' ? 'macOS Apple Silicon DMG' : 'Windows x64 安装包'}`,
          );
        } else {
          button.href = release.url;
          const caption = button.querySelector('small');
          if (caption) caption.textContent = '查看该版本可用附件';
        }
      }
    }
  }
} catch {
  // The HTML already links to the releases page when metadata cannot be loaded.
}
