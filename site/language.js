// Run before styles to restore this browser's explicit language and theme choices.
try {
  const root = document.documentElement;
  const theme = localStorage.getItem('travel-gallery-site-theme');
  root.dataset.theme =
    theme === 'dark' || theme === 'light'
      ? theme
      : matchMedia('(prefers-color-scheme: dark)').matches
        ? 'dark'
        : 'light';
  const saved = localStorage.getItem('travel-gallery-site-language');
  const english = root.lang.startsWith('en');
  if ((saved === 'en-US' && !english) || (saved === 'zh-CN' && english)) {
    const base = new URL('./', document.currentScript.src);
    const page = location.pathname.endsWith('guide.html') ? 'guide.html' : 'index.html';
    const destination = new URL(`${saved === 'en-US' ? 'en/' : ''}${page}`, base);
    destination.hash = location.hash;
    location.replace(destination);
  }
} catch {
  // Explicit language links remain usable when storage is unavailable.
}
document.addEventListener('click', (event) => {
  const link = event.target instanceof Element ? event.target.closest('[data-language]') : null;
  if (!link) return;
  const url = new URL(link.href);
  url.hash = location.hash;
  link.href = url.href;
  try {
    localStorage.setItem('travel-gallery-site-language', link.dataset.language);
  } catch {
    /* Navigation does not require storage. */
  }
});
