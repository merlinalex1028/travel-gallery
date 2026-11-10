import { mkdir, writeFile } from 'node:fs/promises';
await mkdir(new URL('../public/demo/', import.meta.url), { recursive: true });
const palettes = [
  ['#90b7c0', '#dbe2cb', '#5c7c70', '#263d3a'],
  ['#aec4cb', '#ebe7da', '#827e72', '#3a4a45'],
  ['#96c7d9', '#d8e4d3', '#759481', '#354f47'],
  ['#c0cbd0', '#ecebe4', '#8d9998', '#3c5455'],
  ['#ddb892', '#f1d6b5', '#9b896a', '#48544c'],
  ['#9cb3bf', '#d6dcce', '#6e827a', '#30443d'],
  ['#89aeb5', '#ccdcd4', '#497267', '#244844'],
  ['#b5bbc8', '#eee6db', '#919298', '#52575c'],
  ['#9cae99', '#d9dfbf', '#7c906d', '#3a5945'],
  ['#b8cec9', '#e7e9d5', '#91a68c', '#4e6c57'],
  ['#a3b1be', '#dbdedc', '#798995', '#3c4c60'],
  ['#d3c0a4', '#e6d8bb', '#a48e6b', '#5a6954'],
];
for (let i = 0; i < 12; i++) {
  const p = palettes[i],
    lake = i % 3 === 0,
    sun = i % 4 === 0;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 800"><defs><linearGradient id="sky" x2="0" y2="1"><stop stop-color="${p[0]}"/><stop offset="1" stop-color="${p[1]}"/></linearGradient><linearGradient id="land" x2="0.2" y2="1"><stop stop-color="${p[2]}"/><stop offset="1" stop-color="${p[3]}"/></linearGradient><filter id="grain"><feTurbulence baseFrequency=".7" numOctaves="3" seed="${i}" stitchTiles="stitch"/><feColorMatrix type="saturate" values="0"/><feComponentTransfer><feFuncA type="linear" slope=".11"/></feComponentTransfer><feBlend in="SourceGraphic" mode="multiply"/></filter></defs><g filter="url(#grain)"><path fill="url(#sky)" d="M0 0h1200v800H0z"/>${sun ? '<circle cx="850" cy="155" r="63" fill="#ffefd2" opacity=".85"/>' : ''}<path fill="${p[2]}" opacity=".4" d="M0 420 135 290 246 370 430 140 575 350 735 210 990 390 1200 260V800H0Z"/><path fill="#e7e9df" opacity=".86" d="m430 140-85 104 79-20 32 37 50-24Zm305 70-65 71 67-13 33 22 21-27Z"/><path fill="url(#land)" d="M0 ${480 - i * 5} 230 397 372 498 645 348 873 510 1060 382 1200 430V800H0Z"/>${lake ? `<path fill="${p[0]}" d="M0 610q280-80 560 5t640-5v190H0Z"/><path fill="${p[1]}" opacity=".25" d="M200 680q300-40 700 0l-80 8H250Z"/>` : `<path fill="${p[3]}" d="M0 690q300-170 600-40t600-15v165H0Z"/><path fill="${p[1]}" opacity=".55" d="m740 800-60-20-50-40 55-50-8-20 40-27-20 31 10 19-48 47 32 25Z"/>`}</g></svg>`;
  await writeFile(new URL(`../public/demo/landscape-${i}.svg`, import.meta.url), svg);
}
