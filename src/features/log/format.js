const MS_PER_SECOND = 1000;

export function formatDuration(durationMs) {
  if (durationMs === null || durationMs === undefined) return "";
  if (durationMs < MS_PER_SECOND) return `${Math.round(durationMs)} ms`;
  return `${(durationMs / MS_PER_SECOND).toFixed(2)} s`;
}

export function formatTime(date) {
  return [date.getHours(), date.getMinutes(), date.getSeconds()]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
}
