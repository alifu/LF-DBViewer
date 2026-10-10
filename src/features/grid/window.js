/**
 * Which rows to draw for a scroll position: the visible ones plus `overscan` on each
 * side, so fast scrolling does not flash blank rows. `end` is exclusive.
 */
export function visibleRange({ scrollTop, viewportHeight, rowHeight, rowCount, overscan = 0 }) {
  if (rowCount <= 0 || rowHeight <= 0) return { start: 0, end: 0 };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const visible = Math.ceil(viewportHeight / rowHeight) + 1;
  const start = Math.min(rowCount, Math.max(0, first - overscan));
  const end = Math.min(rowCount, first + visible + overscan);
  return { start, end };
}
