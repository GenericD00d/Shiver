/** Sharkord's full-screen picture, made to pinch and pan with a finger. */

import { ensureStyle } from '../../shared/web/bridge/dom';
import { IMAGE_VIEWER } from '../../shared/web/bridge/sharkord';

const MAX_ZOOM = 6;
/** a second tap this soon (and this close) after the first is a double tap */
const DOUBLE_TAP_MS = 300;
const DOUBLE_TAP_ZOOM = 2.5;

/**
 * Sharkord's full-screen picture zooms with the mouse wheel and pans by mouse drag only. Here it
 * pinches, pans with a finger once zoomed, and double-taps in and out. The viewer's touches go no
 * further, since they would otherwise reach Sharkord's swipes (through the portal) and open the
 * drawer behind it.
 */
export function installImageZoom() {
  ensureStyle('shiver-image-zoom').textContent = `${IMAGE_VIEWER} img { touch-action: none; }`;

  type Point = { x: number; y: number };

  let image: HTMLImageElement | null = null;
  /** Sharkord's opening size, which is also the smallest */
  let base = 1;
  let scale = 1;
  let shift: Point = { x: 0, y: 0 };
  /** the gesture in progress, from where it started */
  let start: { points: Point[]; scale: number; shift: Point; centre: Point; at: number } | null = null;
  let lastTap: { at: number; point: Point } | null = null;

  const pointsOf = (touches: TouchList) => [...touches].map((touch) => ({ x: touch.clientX, y: touch.clientY }));
  const middle = (points: Point[]) => ({
    x: points.reduce((sum, point) => sum + point.x, 0) / points.length,
    y: points.reduce((sum, point) => sum + point.y, 0) / points.length
  });
  const apart = ([a, b]: Point[]) => Math.hypot(a.x - b.x, a.y - b.y);

  const apply = () => {
    if (image) image.style.transform = `translate(${shift.x}px, ${shift.y}px) scale(${scale})`;
  };

  /** Scales to `next`, keeping what was under `from` (at the gesture's start) under `to`. */
  const zoomAbout = (next: number, from: Point, to: Point) => {
    if (!start) return;

    const factor = next / start.scale;

    scale = next;
    shift = {
      x: to.x - start.centre.x - (from.x - start.centre.x - start.shift.x) * factor,
      y: to.y - start.centre.y - (from.y - start.centre.y - start.shift.y) * factor
    };
  };

  const begin = (touches: TouchList) => {
    if (!image) return;

    const rect = image.getBoundingClientRect();

    start = {
      points: pointsOf(touches),
      scale,
      shift: { ...shift },
      // the untransformed centre, which scaling is about
      centre: { x: rect.left + rect.width / 2 - shift.x, y: rect.top + rect.height / 2 - shift.y },
      at: Date.now()
    };
  };

  const ownTouch = (event: TouchEvent) => event.target instanceof Element && !!event.target.closest(IMAGE_VIEWER);

  window.addEventListener(
    'touchstart',
    (event) => {
      if (!ownTouch(event)) return;

      event.stopPropagation();

      const target = event.target instanceof HTMLImageElement ? event.target : null;

      if (!target) return;

      if (target !== image) {
        image = target;
        base = Number(/scale\(([\d.]+)\)/.exec(target.style.transform)?.[1]) || 1;
        scale = base;
        shift = { x: 0, y: 0 };
      }

      begin(event.touches);
    },
    { capture: true, passive: true }
  );

  window.addEventListener(
    'touchmove',
    (event) => {
      if (!ownTouch(event)) return;

      event.stopPropagation();

      if (!start || !image) return;

      const points = pointsOf(event.touches);

      if (points.length >= 2 && start.points.length >= 2) {
        const next = Math.min(Math.max((start.scale * apart(points)) / apart(start.points), base * 0.5), MAX_ZOOM);

        zoomAbout(next, middle(start.points), middle(points));
      } else if (points.length === 1 && scale > base) {
        shift = { x: start.shift.x + points[0].x - start.points[0].x, y: start.shift.y + points[0].y - start.points[0].y };
      }

      apply();
    },
    { capture: true, passive: true }
  );

  const end = (event: TouchEvent) => {
    if (!ownTouch(event)) return;

    event.stopPropagation();

    if (!start || !image) return;

    const tapped = event.touches.length === 0 && start.points.length === 1 && Date.now() - start.at < DOUBLE_TAP_MS;
    const point = start.points[0];

    if (tapped && lastTap && Date.now() - lastTap.at < DOUBLE_TAP_MS && Math.hypot(point.x - lastTap.point.x, point.y - lastTap.point.y) < 40) {
      lastTap = null;

      if (scale > base) {
        scale = base;
        shift = { x: 0, y: 0 };
      } else {
        zoomAbout(DOUBLE_TAP_ZOOM, point, point);
      }
    } else if (tapped) {
      lastTap = { at: Date.now(), point };
    }

    // a pinch let out below the opening size springs back to it
    if (scale < base) {
      scale = base;
      shift = { x: 0, y: 0 };
    }

    apply();

    // a finger still down carries on as a pan from here
    if (event.touches.length) begin(event.touches);
    else start = null;
  };

  window.addEventListener('touchend', end, { capture: true, passive: true });
  window.addEventListener('touchcancel', end, { capture: true, passive: true });
}
