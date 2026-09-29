export const PROTOCOL_VERSION = 1;

const ALLOWED = new Set([
  "cookie",
  "referer",
  "origin",
  "user-agent",
  "authorization",
  "accept",
  "accept-language",
  "accept-encoding",
]);

function blocked(name) {
  const n = String(name || "").toLowerCase();
  return (
    n === "host" ||
    n === "connection" ||
    n === "keep-alive" ||
    n === "proxy-connection" ||
    n === "te" ||
    n === "trailer" ||
    n === "transfer-encoding" ||
    n === "upgrade" ||
    n === "content-length" ||
    n === "content-encoding" ||
    n.startsWith("sec-") ||
    n.startsWith(":")
  );
}

function canonical(name) {
  const n = String(name || "").toLowerCase();
  const map = {
    cookie: "Cookie",
    referer: "Referer",
    origin: "Origin",
    "user-agent": "User-Agent",
    authorization: "Authorization",
    accept: "Accept",
    "accept-language": "Accept-Language",
    "accept-encoding": "Accept-Encoding",
  };
  return map[n] || name;
}

export function filterHeaders(input) {
  const out = {};
  if (!input) return out;
  const entries = Array.isArray(input)
    ? input.map((h) => [h.name, h.value])
    : Object.entries(input);
  for (const [k, v] of entries) {
    if (!k || v == null || v === "") continue;
    if (blocked(k)) continue;
    if (!ALLOWED.has(String(k).toLowerCase())) continue;
    out[canonical(k)] = String(v);
  }
  return out;
}

export function buildDownloadRequest(partial) {
  const headers = filterHeaders(partial.headers || {});
  if (partial.cookies && !headers.Cookie) headers.Cookie = partial.cookies;
  if (partial.referrer && !headers.Referer) headers.Referer = partial.referrer;
  if (partial.userAgent && !headers["User-Agent"]) headers["User-Agent"] = partial.userAgent;
  return {
    protocol_version: PROTOCOL_VERSION,
    request_id: partial.requestId || crypto.randomUUID(),
    action: partial.action || "add",
    url: partial.url || "",
    final_url: partial.finalUrl || partial.url || "",
    filename: partial.filename || "",
    method: partial.method || "GET",
    referrer: partial.referrer || "",
    user_agent: partial.userAgent || "",
    cookies: partial.cookies || headers.Cookie || "",
    headers,
    tab_url: partial.tabUrl || "",
    content_type: partial.contentType || "",
    content_length: Number(partial.contentLength || 0) || 0,
    proxy_name: partial.proxyName || "",
    connections: partial.connections ?? 0,
  };
}

export function parseAck(data) {
  if (data == null) return { ok: false, accepted: false, reason: "empty ack" };
  let parsed = data;
  if (typeof data === "string") {
    try {
      parsed = JSON.parse(data);
    } catch {
      return { ok: false, accepted: false, reason: "non-json ack" };
    }
  }
  if (typeof parsed !== "object") return { ok: false, accepted: false, reason: "invalid ack" };
  if (parsed.accepted === true) return { ok: true, accepted: true, reason: "", requestId: parsed.request_id || "" };
  if (parsed.status === "ok" && parsed.accepted !== false) {
    return { ok: true, accepted: true, reason: "legacy", requestId: parsed.request_id || "" };
  }
  return {
    ok: false,
    accepted: false,
    reason: parsed.reason || "rejected",
    requestId: parsed.request_id || "",
  };
}

export function mediaDedupKey(url, contentType) {
  try {
    const u = new URL(url);
    u.hash = "";
    return `${u.origin}${u.pathname}|${contentType || ""}`;
  } catch {
    return `${url}|${contentType || ""}`;
  }
}

export function shouldSkipMediaUrl(url) {
  if (!url) return true;
  if (url.startsWith("blob:") || url.startsWith("data:")) return true;
  const lower = url.toLowerCase();
  if (/\.ts(\?|$)/.test(lower) && !lower.includes(".m3u8")) return true;
  return false;
}

function extensionOf(filename, url) {
  let path = filename || "";
  if (!path) {
    try {
      path = new URL(url).pathname;
    } catch {
      path = url || "";
    }
  }
  const ext = path.split(".").pop()?.toLowerCase() || "";
  if (!ext || /[\\/]/.test(ext)) return "";
  return ext;
}

// Why a browser download must be left alone. Empty string means it can be taken.
export function ignoreReason(url, filename, fileSize, settings) {
  const cfg = settings || {};
  let hostname = "";
  try {
    hostname = new URL(url).hostname;
  } catch {
    return "";
  }
  const ignored = String(cfg.ignoredDomains || "")
    .split(/[,\s]+/)
    .filter(Boolean);
  if (ignored.some((d) => hostname === d || hostname.endsWith("." + d))) return "ignored-domain";
  const ext = extensionOf(filename, url);
  const ignoredExt = String(cfg.ignoredExtensions || "")
    .split(/[,\s]+/)
    .filter(Boolean)
    .map((s) => s.replace(/^\./, "").toLowerCase());
  if (ext && ignoredExt.includes(ext)) return "ignored-extension";
  const minSize = Number(cfg.minSize || 0);
  const size = Number(fileSize || 0);
  if (minSize > 0 && size > 0 && size < minSize) return "min-size";
  return "";
}

// Fast-path decision. Reads only the values the caller already has in memory.
export function interceptDecision(input) {
  const src = input || {};
  if (!src.enabled) return { take: false, reason: "disabled" };
  if (!src.connected) return { take: false, reason: "offline" };
  const url = src.url || "";
  if (!url) return { take: false, reason: "empty-url" };
  if (url.startsWith("blob:")) return { take: false, reason: "blob" };
  if (url.startsWith("data:")) return { take: false, reason: "data" };
  if (src.restored) return { take: false, reason: "restored" };
  if (src.bypass) return { take: false, reason: "bypass" };
  const ignored = ignoreReason(url, src.filename || "", src.fileSize, src.settings);
  if (ignored) return { take: false, reason: ignored };
  return { take: true, reason: "take" };
}
