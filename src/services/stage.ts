export type Pan = { x: number; y: number };
/** Pan is a fraction of the rendered image, so panels align the same image location. */
export function stageGeometry(
  view: { width: number; height: number },
  image: { width: number; height: number },
  zoom: number,
  actual: boolean,
  pan: Pan,
) {
  const fit = actual ? 1 : Math.min(view.width / image.width, view.height / image.height);
  const width = Math.max(1, image.width * fit * zoom),
    height = Math.max(1, image.height * fit * zoom);
  const maxX = Math.max(0, (width - view.width) / (2 * width));
  const maxY = Math.max(0, (height - view.height) / (2 * height));
  const normalized = { x: Math.max(-maxX, Math.min(maxX, pan.x)), y: Math.max(-maxY, Math.min(maxY, pan.y)) };
  return { width, height, x: normalized.x * width, y: normalized.y * height, normalized };
}
