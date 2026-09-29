import type { DownloadStatus } from "../types";
import { isActiveStatus, statusString } from "./format";

export interface RuntimeControlCapabilities {
  proxy: boolean;
  connections: boolean;
  rateLimit: boolean;
}

export function isHlsItem(contentType: string | undefined, fileName: string): boolean {
  const ct = (contentType || "").toLowerCase();
  return ct.includes("mpegurl") || fileName.toLowerCase().endsWith(".m3u8");
}

const NONE: RuntimeControlCapabilities = { proxy: false, connections: false, rateLimit: false };

/** Mirrors the backend: a control is on only when that engine mode will apply it. */
export function runtimeControlCapabilities(item: {
  status: DownloadStatus;
  resumable: boolean | null;
  content_type?: string;
  file_name: string;
}): RuntimeControlCapabilities {
  const status = statusString(item.status);
  if (status === "completed" || status === "failed") return { ...NONE };
  if (status === "paused") return { proxy: true, connections: true, rateLimit: true };
  const hls = isHlsItem(item.content_type, item.file_name);
  if (status === "queued") {
    return {
      proxy: true,
      connections: hls || item.resumable !== false,
      rateLimit: true,
    };
  }
  if (!isActiveStatus(item.status)) return { ...NONE };
  if (hls || item.resumable === false) return { proxy: false, connections: false, rateLimit: true };
  return { proxy: true, connections: true, rateLimit: true };
}

export function proxySelectLabel(name: string, noProxy: string): string {
  return name ? name : noProxy;
}

export function connectionOptionLabel(connections: number, autoLabel: string): string {
  return connections === 0 ? autoLabel : String(connections);
}
