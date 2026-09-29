/** ".PDF" and "Report.PDF" → ".pdf". Names without an extension → "". */
export function normalizeExtension(name: string): string {
  const base = name.split(/[/\\]/).pop() ?? name;
  const trimmed = base.trim();
  const dot = trimmed.lastIndexOf(".");
  if (dot <= 0) return dot === 0 ? trimmed.toLowerCase() : "";
  return trimmed.slice(dot).toLowerCase();
}

/**
 * Client cache key. Completed files follow the save path so a rename or a
 * finished download does not keep the in-progress type icon. The backend
 * adds mtime and size on top of this.
 */
export function clientIconKey(fileName: string, path: string, completed: boolean): string {
  if (completed && path) return `file:${path}`;
  const ext = normalizeExtension(fileName);
  return `type:${ext || "none"}`;
}
