export function formatDate(dateStr) {
  return new Date(dateStr + "Z").toLocaleString("pt-BR");
}

export function formatDuration(secs) {
  const m = Math.floor(secs / 60);
  const s = Math.round(secs % 60);
  return `${m}min ${s}s`;
}
