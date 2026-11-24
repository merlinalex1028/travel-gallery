import { readFileSync } from 'node:fs';
import { expect, it } from 'vitest';

const css = readFileSync('src/theme.css', 'utf8');
function palette(selector: string) {
  const block = css.slice(css.indexOf(`${selector} {`)).split('}')[0]!;
  return Object.fromEntries(
    [...block.matchAll(/--([\w-]+):\s*(#[\da-f]+);/gi)].map((match) => [match[1]!, match[2]!]),
  );
}
function luminance(hex: string) {
  const value = hex.slice(1);
  const rgb = value.length === 3 ? [...value].map((digit) => digit.repeat(2)).join('') : value;
  return [0, 2, 4]
    .map((offset) => parseInt(rgb.slice(offset, offset + 2), 16) / 255)
    .map((channel) => (channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4))
    .reduce((sum, channel, index) => sum + channel * [0.2126, 0.7152, 0.0722][index]!, 0);
}
function contrast(first: string, second: string) {
  const values = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (values[0]! + 0.05) / (values[1]! + 0.05);
}

it('keeps dark primary and secondary form text above AAA contrast', () => {
  const dark = palette(':root.dark');
  expect(contrast(dark.text!, dark.surface!)).toBeGreaterThanOrEqual(11);
  for (const surface of ['surface', 'surface-raised', 'sidebar', 'accent-soft']) {
    expect(contrast(dark.text!, dark[surface]!)).toBeGreaterThanOrEqual(7);
    expect(contrast(dark['text-secondary']!, dark[surface]!)).toBeGreaterThanOrEqual(7);
  }
});

it.each([':root', ':root.dark'])('keeps %s hints and accent button text readable', (selector) => {
  const colors = { ...palette(':root'), ...palette(selector) };
  for (const surface of ['surface', 'sidebar', 'accent-soft']) {
    expect(contrast(colors.muted!, colors[surface]!)).toBeGreaterThanOrEqual(4.5);
  }
  if (selector.endsWith('.dark'))
    expect(contrast(colors.muted!, colors['surface-raised']!)).toBeGreaterThanOrEqual(4.5);
  expect(contrast(colors['on-accent']!, colors.accent!)).toBeGreaterThanOrEqual(4.5);
});
