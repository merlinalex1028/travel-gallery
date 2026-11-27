const root = document.documentElement;
const english = root.lang.startsWith('en');
const copy = (zh, en) => (english ? en : zh);
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
  document.querySelector('meta[name="theme-color"]').content = getComputedStyle(root)
    .getPropertyValue('--background')
    .trim();
  themeButton.setAttribute('aria-pressed', String(theme === 'dark'));
  themeButton.setAttribute(
    'aria-label',
    theme === 'dark'
      ? copy('切换为浅色主题', 'Switch to light theme')
      : copy('切换为深色主题', 'Switch to dark theme'),
  );
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
  keep: copy('已保留 · 这个瞬间值得', 'Kept · A moment worth keeping'),
  reject: copy('已标记淘汰 · 还可以再想想', 'Flagged as rejected · You can still reconsider'),
  favorite: copy('已收藏 · 留住这份心动', 'Favorited · Keep that feeling'),
};
function review(action) {
  const next = memory.dataset.state === action ? 'unreviewed' : action;
  memory.dataset.state = next;
  reviewStatus.textContent = labels[next] ?? copy('待你发现', 'Waiting to be discovered');
  for (const button of reviewButtons)
    button.setAttribute('aria-pressed', String(button.dataset.review === next));
}
for (const button of reviewButtons) {
  button.addEventListener('click', () => review(button.dataset.review));
}
document.querySelector('.review-controls')?.addEventListener('keydown', (event) => {
  if (event.ctrlKey || event.metaKey || event.altKey || event.repeat) return;
  const action = { p: 'keep', x: 'reject', f: 'favorite' }[event.key.toLowerCase()];
  if (action) {
    event.preventDefault();
    review(action);
  }
});

// Download metadata is generated during deployment; no visitor-side GitHub API call is needed.
const downloadFallback = new URL(`${english ? './en/' : './'}guide.html#versions`, import.meta.url).href;
for (const button of document.querySelectorAll('[data-download]')) button.href = downloadFallback;
try {
  const response = await fetch(new URL('./downloads.json', import.meta.url));
  if (!response.ok) throw new Error('Download metadata unavailable');
  const release = await response.json();
  const history = document.querySelector('[data-release-history]');
  if (history && release.releases?.length) {
    history.replaceChildren();
    for (const entry of release.releases) {
      if (!entry.url.startsWith('https://github.com/merlinalex1028/travel-gallery/releases/tag/')) continue;
      const article = document.createElement('article');
      article.className = 'release-entry';
      const title = document.createElement('h3');
      title.textContent = entry.name || entry.tag;
      const date = document.createElement('p');
      date.textContent = `${new Intl.DateTimeFormat(english ? 'en-US' : 'zh-CN', { dateStyle: 'medium' }).format(new Date(entry.publishedAt))} · ${entry.prerelease ? copy('预发布体验版', 'Prerelease') : copy('正式发布', 'Stable release')}`;
      const link = document.createElement('a');
      link.href = entry.url;
      link.textContent = copy('在 GitHub 查看此版本及校验清单', 'View this release and checksums on GitHub');
      article.append(title, date, link);
      history.append(article);
    }
  }
  if (release.tag) {
    for (const status of document.querySelectorAll('[data-release-status]'))
      status.textContent = `${release.tag} · ${release.prerelease ? copy('预发布体验版', 'Prerelease') : copy('正式发布', 'Stable release')}`;
    for (const platform of ['macos', 'windows']) {
      const file = release.assets[platform];
      for (const button of document.querySelectorAll(`[data-download="${platform}"]`)) {
        if (file) {
          button.href = file.url;
          button.setAttribute(
            'aria-label',
            `${copy('下载', 'Download')} ${release.tag} ${platform === 'macos' ? 'macOS Apple Silicon DMG' : copy('Windows x64 安装包', 'Windows x64 installer')}`,
          );
        } else {
          button.href = downloadFallback;
          const caption = button.querySelector('small');
          if (caption) caption.textContent = copy('查看该版本可用附件', 'View available installers');
        }
      }
    }
  }
} catch {
  // Keep ordinary navigation on-site; the guide explicitly labels the GitHub fallback.
  for (const button of document.querySelectorAll('[data-download]')) {
    button.href = downloadFallback;
  }
}
