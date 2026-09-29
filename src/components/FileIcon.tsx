import { useEffect, useState, useSyncExternalStore } from "react";
import { EVENTS } from "../constants/events";
import { tauriClient, type FileIconBatch, type FileIconRequest } from "../tauriClient";
import { clientIconKey } from "../utils/fileIcon";

const GENERIC =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none">
      <path d="M7 3.5h7.2L18 7.2V20a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 6 20V5A1.5 1.5 0 0 1 7 3.5Z" stroke="#a3a3a3" stroke-width="1.6"/>
      <path d="M14 3.8V7.5h3.6" stroke="#a3a3a3" stroke-width="1.6"/>
    </svg>`,
  );

const cache = new Map<string, string>();
const keysById = new Map<number, Set<string>>();
const generations = new Map<number, number>();
const listeners = new Set<() => void>();

type Waiter = (url: string) => void;
const waiters = new Map<string, Waiter[]>();
let queued: FileIconRequest[] = [];
let flushTimer: ReturnType<typeof setTimeout> | null = null;
let hooked = false;

function bump(id: number) {
  generations.set(id, (generations.get(id) ?? 0) + 1);
  listeners.forEach((listener) => listener());
}

function dropId(id: number) {
  const keys = keysById.get(id);
  if (!keys) return;
  for (const key of keys) cache.delete(key);
  keysById.delete(id);
}

function remember(id: number, key: string) {
  let keys = keysById.get(id);
  if (!keys) {
    keys = new Set();
    keysById.set(id, keys);
  }
  keys.add(key);
}

function ensureCompletedHook() {
  if (hooked) return;
  hooked = true;
  void import("@tauri-apps/api/event")
    .then(({ listen }) =>
      listen<{ id: number }>(EVENTS.DOWNLOAD_COMPLETED, (event) => {
        const id = event.payload?.id;
        if (typeof id !== "number") return;
        dropId(id);
        bump(id);
      }),
    )
    .catch(() => {});
}

function dataUrl(batch: FileIconBatch, backendKey: string): string {
  const icon = batch.icons.find((item) => item.key === backendKey);
  if (!icon?.data) return "";
  return icon.data.startsWith("data:") ? icon.data : `data:image/png;base64,${icon.data}`;
}

async function flush() {
  flushTimer = null;
  const batch = queued;
  queued = [];
  if (batch.length === 0) return;
  const seen = new Set<string>();
  const requests: FileIconRequest[] = [];
  for (const req of batch) {
    const key = clientIconKey(req.fileName, req.path, req.completed);
    if (seen.has(key)) continue;
    seen.add(key);
    requests.push(req);
  }
  let result: FileIconBatch | null = null;
  try {
    result = await tauriClient.getFileIcons(requests);
  } catch {
    result = null;
  }
  const resolved = new Set<string>();
  if (result) {
    for (const match of result.matches) {
      const req = requests.find((item) => item.id === match.id);
      if (!req) continue;
      const key = clientIconKey(req.fileName, req.path, req.completed);
      const url = dataUrl(result, match.key);
      if (url) cache.set(key, url);
      resolved.add(key);
      const pending = waiters.get(key) ?? [];
      waiters.delete(key);
      pending.forEach((resolve) => resolve(url));
    }
  }
  for (const req of requests) {
    const key = clientIconKey(req.fileName, req.path, req.completed);
    if (resolved.has(key)) continue;
    const pending = waiters.get(key) ?? [];
    waiters.delete(key);
    pending.forEach((resolve) => resolve(""));
  }
}

function loadFileIcon(req: FileIconRequest): Promise<string> {
  ensureCompletedHook();
  const key = clientIconKey(req.fileName, req.path, req.completed);
  remember(req.id, key);
  const hit = cache.get(key);
  if (hit) return Promise.resolve(hit);
  return new Promise((resolve) => {
    const pending = waiters.get(key) ?? [];
    pending.push(resolve);
    waiters.set(key, pending);
    if (pending.length === 1) queued.push(req);
    if (flushTimer == null) flushTimer = setTimeout(() => void flush(), 16);
  });
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

interface FileIconProps {
  id: number;
  fileName: string;
  path?: string;
  mimeType?: string;
  completed: boolean;
  size: number;
}

/** System file icon. Shows a generic page immediately, then the cached PNG. */
export default function FileIcon({ id, fileName, path = "", mimeType = "", completed, size }: FileIconProps) {
  const generation = useSyncExternalStore(
    subscribe,
    () => generations.get(id) ?? 0,
    () => 0,
  );
  const [src, setSrc] = useState(GENERIC);

  useEffect(() => {
    let alive = true;
    void loadFileIcon({
      id,
      fileName,
      path,
      mimeType,
      completed,
    }).then((url) => {
      if (alive && url) setSrc(url);
    });
    return () => {
      alive = false;
    };
  }, [id, fileName, path, mimeType, completed, generation]);

  return (
    <img
      src={src}
      alt=""
      width={size}
      height={size}
      draggable={false}
      className="shrink-0 object-contain"
      style={{ width: size, height: size }}
    />
  );
}
