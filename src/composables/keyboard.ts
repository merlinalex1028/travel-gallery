import { onMounted, onUnmounted } from 'vue';
export function hasActiveModal(): boolean {
  return [
    ...document.querySelectorAll<HTMLElement>(
      '.el-overlay-dialog, [data-modal="true"], [role="menu"], [role="listbox"]',
    ),
  ].some((element) => {
    for (let node: HTMLElement | null = element; node; node = node.parentElement) {
      const style = getComputedStyle(node);
      if (
        node.hidden ||
        node.inert ||
        node.getAttribute('aria-hidden') === 'true' ||
        style.display === 'none' ||
        style.visibility === 'hidden' ||
        style.visibility === 'collapse'
      )
        return false;
    }
    return true;
  });
}
export function isMediaKeyboardBlocked(event: KeyboardEvent): boolean {
  if (event.defaultPrevented || event.isComposing) return true;
  const targets = [event.target, ...event.composedPath()];
  return (
    hasActiveModal() ||
    targets.some(
      (target) =>
        target instanceof Element &&
        target.closest(
          'input,textarea,select,video,audio,[contenteditable]:not([contenteditable="false"]),[role="dialog"],[role="menu"],[role="listbox"],[role="combobox"],[role="slider"]',
        ),
    )
  );
}
export function photoKey(event: KeyboardEvent): string | null {
  if (isMediaKeyboardBlocked(event)) return null;
  if (
    event.key === ' ' &&
    [event.target, ...event.composedPath()].some(
      (target) => target instanceof Element && target.closest('button,a,[role="button"]'),
    )
  )
    return null;
  if (event.altKey) return null;
  if (event.metaKey || event.ctrlKey) {
    if (event.shiftKey || event.repeat) return null;
    const key = event.key.toLowerCase();
    return key === 'z' ? 'undo' : key === 'a' ? 'select-all' : null;
  }
  if (event.repeat && !['ArrowLeft', 'ArrowRight'].includes(event.key)) return null;
  const key = event.key.toLowerCase();
  return [
    'p',
    'x',
    'u',
    'f',
    '0',
    '1',
    '2',
    '3',
    '4',
    '5',
    'arrowleft',
    'arrowright',
    ' ',
    'tab',
    'escape',
  ].includes(key)
    ? key
    : null;
}
export function useMediaKeyboard(handler: (key: string, event: KeyboardEvent) => void) {
  function listener(event: KeyboardEvent) {
    const key = photoKey(event);
    if (key) handler(key, event);
  }
  onMounted(() => document.addEventListener('keydown', listener));
  onUnmounted(() => document.removeEventListener('keydown', listener));
}
