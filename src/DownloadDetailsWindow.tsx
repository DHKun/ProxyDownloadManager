import { useEffect, useState } from "react";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { errorDetail, failureText, formatBytes, formatRateLimit, isActiveStatus, isFailed, statusLabel, statusString } from "./utils/format";
import { useDownloadDetail, useDownloadIdFromUrl } from "./hooks/useDownloadDetail";
import { useDownloadSpeed, computeETA } from "./hooks/useDownloadSpeed";
import { useDeleteDownload, useRedownloadDownload, useSettings } from "./query/downloadQueries";
import { connectionSegments, overallPercent } from "./utils/progressMap";
import { setLanguage, t } from "./i18n";
import { Button } from "./components/ui/button";
import { Progress } from "./components/ui/progress";
import { Select } from "./components/ui/select";
import { tauriClient } from "./tauriClient";
import type { DownloadItem } from "./types";

const CONN_OPTIONS = [0, 1, 4, 8, 16, 32, 64];
const RATE_OPTIONS = [0, 256 * 1024, 1024 * 1024, 5 * 1024 * 1024, 10 * 1024 * 1024];
const DETAILS_SIZE = { width: 560, height: 320 };

interface Draft {
  id: number;
  conns: number;
  rate: string;
  proxy: string;
}

function withCurrent(options: number[], current: number): number[] {
  return options.includes(current) ? options : [...options, current].sort((a, b) => a - b);
}

/** Same buckets as engine/chunk.rs auto_connections, for the Auto · n label before parts exist. */
function autoConnectionCount(fileSize: number): number {
  const mib = 1024 * 1024;
  if (fileSize <= 0) return 2;
  if (fileSize < 2 * mib) return 1;
  if (fileSize < 16 * mib) return 4;
  if (fileSize < 128 * mib) return 8;
  if (fileSize < 1024 * mib) return 16;
  return 32;
}

function liveConnectionCount(item: DownloadItem): number {
  if (item.parts.length > 0) return item.parts.length;
  if (item.connections > 0) return item.connections;
  return autoConnectionCount(item.total_size);
}

function isSegmentItem(contentType: string | undefined, fileName: string): boolean {
  const ct = (contentType || "").toLowerCase();
  return ct.includes("mpegurl") || fileName.toLowerCase().endsWith(".m3u8");
}

function compactUrl(url: string): string {
  return url.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "");
}

function rateText(bps: number): string {
  return bps > 0 ? formatRateLimit(bps) : t("rateLimit.unlimited");
}

function Field({ label, value, title, valueClass = "" }: {
  label: string;
  value: string;
  title?: string;
  valueClass?: string;
}) {
  return (
    <div className="flex min-w-0 items-baseline gap-1.5">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className={`min-w-0 truncate whitespace-nowrap ${valueClass}`} title={title ?? value}>{value}</span>
    </div>
  );
}

export default function DownloadDetailsWindow() {
  const idParam = new URLSearchParams(window.location.search).get("id");
  const urlId = useDownloadIdFromUrl();
  const [liveId, setLiveId] = useState(urlId);
  const { settings: loadedSettings } = useSettings();
  const [, bumpLang] = useState(0);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [extraBusy, setExtraBusy] = useState(false);
  const removeDownload = useDeleteDownload();
  const redownload = useRedownloadDownload();

  useEffect(() => {
    if (loadedSettings) {
      setLanguage(loadedSettings.language || "en");
      bumpLang((n) => n + 1);
    }
  }, [loadedSettings]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      unlisten = await listen<number>("details-id", (e) => setLiveId(e.payload));
    })();
    return () => { unlisten?.(); };
  }, []);

  useEffect(() => {
    const win = getCurrentWebviewWindow();
    if (typeof win.setSize !== "function") return;
    void win.setSize(new LogicalSize(DETAILS_SIZE.width, DETAILS_SIZE.height)).catch(() => {});
  }, []);

  const id = liveId ?? urlId;
  const {
    item, urlCopied, controls, pendingAction, handleCopyUrl, handleOpenFile, handleOpenFolder, handlePause, handleResume,
  } = useDownloadDetail(id);

  const speedInputs = item && isActiveStatus(item.status) ? [item] : [];
  const speeds = useDownloadSpeed(speedInputs);

  useEffect(() => {
    const name = item?.file_name;
    if (!name) return;
    const win = getCurrentWebviewWindow();
    if (typeof win.setTitle === "function") {
      void win.setTitle(name).catch(() => {});
    }
  }, [item?.file_name]);

  if (!idParam && liveId == null) {
    return <div className="grid h-full place-items-center text-[13px]">No download ID provided</div>;
  }
  if (!item) {
    return <div className="grid h-full place-items-center text-[13px]">{t("downloadTable.loading")}</div>;
  }

  const chosen = draft?.id === item.id ? draft : null;
  const connValue = chosen?.conns ?? item.connections;
  const rateValue = chosen?.rate ?? String(item.rate_limit_bps || 0);
  const proxyValue = chosen?.proxy ?? item.proxy_name ?? "";
  const progress = overallPercent(item.downloaded, item.total_size, item.status);
  const speed = speeds.get(item.id);
  const active = isActiveStatus(item.status);
  const failed = isFailed(item.status) || statusString(item.status) === "failed";
  const completed = statusString(item.status) === "completed";
  const adjustable = !failed && !completed;
  const busy = controls.busy || extraBusy;
  const segments = isSegmentItem(item.content_type, item.file_name);
  const { code, message } = failureText(item.status, item.error_message);
  const detail = errorDetail(code, message);
  const statusText = failed
    ? (code != null ? `${t("status.failed")} · HTTP ${code}` : t("status.failed"))
    : `${statusLabel(item.status)} · ${progress}%`;
  const statusTitle = failed ? (detail || message || undefined) : undefined;
  const liveCount = liveConnectionCount(item);
  const connLabel = item.connections === 0
    ? t("properties.autoLive").replace("{n}", String(liveCount))
    : String(item.connections);
  const slices = connectionSegments(item.parts, item.parts.length > 0 ? 0 : liveCount, item.status);
  const proxies = loadedSettings?.proxies ?? {};
  const proxyNames = Object.keys(proxies);
  const proxyOptions = proxyNames.includes(proxyValue) || proxyValue === ""
    ? proxyNames
    : [proxyValue, ...proxyNames];
  const connOptions = withCurrent(CONN_OPTIONS, connValue);
  const rateBps = Number(rateValue) || 0;
  const rateOptions = withCurrent(withCurrent(RATE_OPTIONS, rateBps), item.rate_limit_bps || 0);
  const sizeText = segments
    ? (item.total_size > 0
      ? t("properties.segments").replace("{done}", String(item.downloaded)).replace("{total}", String(item.total_size))
      : "—")
    : item.total_size
      ? formatBytes(item.total_size)
      : "—";
  const downloadedText = segments ? String(item.downloaded) : formatBytes(item.downloaded);
  const resumeText = item.resumable === false
    ? t("properties.unsupported")
    : item.resumable === true
      ? t("properties.supported")
      : t("properties.unknown");

  const remember = (next: Partial<Pick<Draft, "conns" | "rate" | "proxy">>) => {
    setDraft({
      id: item.id,
      conns: next.conns ?? connValue,
      rate: next.rate ?? rateValue,
      proxy: next.proxy ?? proxyValue,
    });
  };
  const applyConnections = (value: number) => {
    remember({ conns: value });
    void tauriClient.setDownloadConnections(item.id, value);
  };
  const applyRate = (value: string) => {
    remember({ rate: value });
    void tauriClient.setDownloadRateLimit(item.id, Number(value) || 0);
  };
  const applyProxy = (value: string) => {
    remember({ proxy: value });
    void tauriClient.setDownloadProxy(item.id, value);
  };
  const handleCancel = async () => {
    if (busy) return;
    setExtraBusy(true);
    try {
      await removeDownload.mutateAsync({ id: item.id, deleteFile: false });
      await getCurrentWebviewWindow().close();
    } catch (e) {
      console.error("[ProxyDM] details cancel failed:", e);
      setExtraBusy(false);
    }
  };
  const handleRetry = async () => {
    if (busy) return;
    setExtraBusy(true);
    try {
      const newId = await redownload.mutateAsync(item.id);
      if (typeof newId === "number") setLiveId(newId);
    } catch (e) {
      console.error("[ProxyDM] details retry failed:", e);
    } finally {
      setExtraBusy(false);
    }
  };
  const closeWindow = () => { void getCurrentWebviewWindow().close(); };

  const autoOption = t("newDownload.autoSuggested").replace("{n}", String(liveCount));

  return (
    <div className="flex h-full flex-col overflow-hidden bg-background text-[13px]">
      <div className="flex min-h-0 flex-1 flex-col gap-2 px-3 pt-2.5">
        <div className="flex items-center gap-2">
          <div className="min-w-0 flex-1 truncate whitespace-nowrap text-muted-foreground" title={item.url}>
            {compactUrl(item.url)}
          </div>
          <Button size="sm" className="h-7 shrink-0 px-2 text-[12px]" disabled={busy} onClick={handleCopyUrl}>
            {urlCopied ? "OK" : t("properties.copy")}
          </Button>
        </div>

        <div className="grid grid-cols-3 gap-x-3 gap-y-1">
          <Field label={t("properties.status")} value={statusText} title={statusTitle} valueClass={failed ? "text-destructive" : ""} />
          <Field label={t("properties.size")} value={sizeText} />
          <Field label={t("properties.remain")} value={active && speed ? computeETA(item, speed.bps) : "—"} />
          <Field label={t("properties.downloaded")} value={downloadedText} />
          <Field label={t("downloadTable.speed")} value={active ? (speed?.display ?? "—") : "—"} />
          <Field label={t("properties.resumeShort")} value={resumeText} />
          <Field label={t("properties.proxy")} value={proxyValue || t("newDownload.noProxy")} />
          <Field label={t("properties.conn")} value={connLabel} />
          <Field label={t("properties.speedLimit")} value={rateText(rateBps)} />
        </div>

        <div className="flex items-center gap-2">
          <Progress className="h-1.5 flex-1" value={progress} />
          <span className="w-10 shrink-0 text-right text-[12px] tabular">{progress}%</span>
        </div>

        <div>
          <div className="mb-1 text-[12px] text-muted-foreground">
            {t("properties.connectionProgress").replace("{n}", String(slices.length || liveCount))}
          </div>
          {slices.length === 0 ? (
            <div className="h-2 rounded-sm bg-muted" />
          ) : (
            <div className="flex h-2 gap-px overflow-hidden rounded-sm bg-background">
              {slices.map((slice) => {
                const range = slice.end > slice.start
                  ? `${formatBytes(slice.start)} – ${formatBytes(slice.end)}`
                  : "";
                const got = slice.end > slice.start
                  ? `${formatBytes(slice.downloaded)} / ${formatBytes(Math.max(0, slice.end - slice.start))}`
                  : `${slice.percent}%`;
                return (
                  <div
                    key={slice.index}
                    className="h-full min-w-px overflow-hidden bg-muted"
                    style={{ flex: `${slice.weight} 1 0` }}
                    title={[`#${slice.index}`, range, got].filter(Boolean).join("\n")}
                  >
                    <div className="h-full bg-foreground" style={{ width: `${slice.percent}%` }} />
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>

      <div className="mt-2 flex items-center gap-1.5 border-t border-border px-2 py-1.5">
        {adjustable && (
          <>
            <Select className="h-7 w-auto min-w-0 flex-1 px-1.5 text-[12px]" value={proxyValue} onChange={(e) => applyProxy(e.target.value)}>
              <option value="">{t("properties.proxy")} {t("newDownload.noProxy")}</option>
              {proxyOptions.map((name) => (
                <option key={name} value={name}>{t("properties.proxy")} {name}</option>
              ))}
            </Select>
            <Select className="h-7 w-auto min-w-0 flex-1 px-1.5 text-[12px]" value={String(connValue)} onChange={(e) => applyConnections(Number(e.target.value))}>
              {connOptions.map((n) => (
                <option key={n} value={n}>
                  {t("properties.conn")} {n === 0 ? autoOption : n}
                </option>
              ))}
            </Select>
            <Select className="h-7 w-auto min-w-0 flex-1 px-1.5 text-[12px]" value={String(rateBps)} onChange={(e) => applyRate(e.target.value)}>
              {rateOptions.map((bps) => (
                <option key={bps} value={bps}>{rateText(bps)}</option>
              ))}
            </Select>
          </>
        )}
        <div className="ml-auto flex shrink-0 gap-1.5">
          {completed && (
            <>
              <Button className="h-7 px-2.5 text-[12px]" disabled={busy} onClick={async () => { if (await handleOpenFile()) closeWindow(); }}>{t("downloadRow.open")}</Button>
              <Button className="h-7 px-2.5 text-[12px]" disabled={busy} onClick={async () => { if (await handleOpenFolder()) closeWindow(); }}>{t("downloadRow.openFolder")}</Button>
            </>
          )}
          {failed && (
            <>
              <Button className="h-7 px-2.5 text-[12px]" disabled={busy} onClick={handleRetry}>{t("properties.retry")}</Button>
              <Button className="h-7 px-2.5 text-[12px]" disabled={busy} onClick={closeWindow}>{t("properties.close")}</Button>
            </>
          )}
          {adjustable && controls.showPause && pendingAction !== "resume" && (
            <Button className="h-7 px-2.5 text-[12px]" disabled={busy} onClick={handlePause}>{t("downloadRow.pause")}</Button>
          )}
          {adjustable && controls.showResume && pendingAction !== "pause" && (
            <Button className="h-7 px-2.5 text-[12px]" disabled={busy} onClick={handleResume}>{t("toolbar.resume")}</Button>
          )}
          {adjustable && (
            <Button className="h-7 px-2.5 text-[12px]" disabled={busy} onClick={handleCancel}>{t("newDownload.cancel")}</Button>
          )}
        </div>
      </div>
    </div>
  );
}
