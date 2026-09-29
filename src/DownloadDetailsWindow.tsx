import { useEffect, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { failureText, formatBytes, isActiveStatus, isFailed, statusString } from "./utils/format";
import { useDownloadDetail, useDownloadIdFromUrl } from "./hooks/useDownloadDetail";
import { useDownloadSpeed, computeETA } from "./hooks/useDownloadSpeed";
import { useSettings } from "./query/downloadQueries";
import ProgressMap from "./components/ProgressMap";
import { overallPercent } from "./utils/progressMap";
import { setLanguage, t } from "./i18n";
import { Button } from "./components/ui/button";
import { Progress } from "./components/ui/progress";
import { Select } from "./components/ui/select";
import { Input } from "./components/ui/input";
import { Label } from "./components/ui/label";
import { tauriClient } from "./tauriClient";

const CONN_OPTIONS = [0, 1, 4, 8, 16, 32, 64];

function isSegmentItem(contentType: string | undefined, fileName: string): boolean {
  const ct = (contentType || "").toLowerCase();
  return ct.includes("mpegurl") || fileName.toLowerCase().endsWith(".m3u8");
}

export default function DownloadDetailsWindow() {
  const idParam = new URLSearchParams(window.location.search).get("id");
  const urlId = useDownloadIdFromUrl();
  const [liveId, setLiveId] = useState(urlId);
  const { settings: loadedSettings } = useSettings();
  const [, bumpLang] = useState(0);
  const [conns, setConns] = useState<number | null>(null);
  const [rate, setRate] = useState("0");
  const [newUrl, setNewUrl] = useState("");

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

  const id = liveId ?? urlId;
  const {
    item, urlCopied, controls, handleCopyUrl, handleOpenFile, handleOpenFolder, handlePause, handleResume,
  } = useDownloadDetail(id);

  const speedInputs = item && isActiveStatus(item.status) ? [item] : [];
  const speeds = useDownloadSpeed(speedInputs);

  useEffect(() => {
    if (item) {
      setConns(item.connections);
      setRate(String(item.rate_limit_bps || 0));
    }
  }, [item?.id]);

  if (!idParam && liveId == null) return <div className="grid h-screen place-items-center text-[13px]">No download ID provided</div>;
  if (!item) return <div className="grid h-screen place-items-center text-[13px]">Loading...</div>;

  const progress = overallPercent(item.downloaded, item.total_size, item.status);
  const speed = speeds.get(item.id);
  const failed = isFailed(item.status) || statusString(item.status) === "failed";
  const showRefresh = failed || statusString(item.status) === "paused";
  const segments = isSegmentItem(item.content_type, item.file_name);
  const { code, message } = failureText(item.status, item.error_message);
  const sizeText = segments
    ? (item.total_size > 0
      ? t("properties.segments").replace("{done}", String(item.downloaded)).replace("{total}", String(item.total_size))
      : "—")
    : item.total_size
      ? formatBytes(item.total_size)
      : "—";
  const downloadedText = segments
    ? String(item.downloaded)
    : formatBytes(item.downloaded);

  const applyConnections = async (value: number) => {
    setConns(value);
    await tauriClient.setDownloadConnections(item.id, value);
  };
  const applyRate = async (value: string) => {
    setRate(value);
    await tauriClient.setDownloadRateLimit(item.id, Number(value) || 0);
  };
  const refresh = async () => {
    if (!newUrl.trim()) return;
    await tauriClient.refreshDownloadUrl(item.id, newUrl.trim());
    setNewUrl("");
  };

  return (
    <div className="flex h-full flex-col gap-3 overflow-auto p-3 text-[13px]">
      <div className="min-w-0 truncate text-[14px] font-semibold">{item.file_name}</div>
      <div>{statusString(item.status)} · {progress}%</div>
      <Progress className="h-2.5" value={progress} />
      <div className="grid grid-cols-2 gap-x-3 gap-y-1">
        <span className="text-muted-foreground">{t("downloadTable.speed")}</span>
        <span className="text-muted-foreground">{t("properties.eta")}</span>
        <span className="tabular">{speed?.display ?? "—"}</span>
        <span className="tabular">{speed ? computeETA(item, speed.bps) : "—"}</span>
        <span className="text-muted-foreground">{t("properties.size")}</span>
        <span className="text-muted-foreground">{t("properties.downloaded")}</span>
        <span className="tabular">{sizeText}</span>
        <span className="tabular">{downloadedText}</span>
        <span className="text-muted-foreground">{t("properties.proxy")}</span>
        <span className="text-muted-foreground">{t("properties.connections")}</span>
        <span>{item.proxy_name || t("properties.none")}</span>
        <span>{item.connections === 0 ? t("newDownload.auto") : item.connections}</span>
      </div>
      {item.resumable === false && (
        <p className="text-[12px] text-muted-foreground">{t("properties.resumeRestarts")}</p>
      )}
      {failed && (
        <div className="rounded-md border border-border bg-muted p-2 text-[12px]">
          <div className="text-muted-foreground">{t("properties.error")}</div>
          {code != null && <div>HTTP {code}</div>}
          {message && <div className="break-all">{message}</div>}
        </div>
      )}
      <div className="flex flex-wrap gap-1.5">
        {controls.showPause && <Button disabled={controls.busy} onClick={handlePause}>{t("downloadRow.pause")}</Button>}
        {controls.showResume && <Button disabled={controls.busy} onClick={handleResume}>{t("toolbar.resume")}</Button>}
        {controls.showOpen && <Button disabled={controls.busy} onClick={async () => { if (await handleOpenFile()) getCurrentWebviewWindow().close(); }}>{t("downloadRow.open")}</Button>}
        <Button disabled={controls.busy} onClick={async () => { if (await handleOpenFolder()) getCurrentWebviewWindow().close(); }}>{t("downloadRow.openFolder")}</Button>
      </div>
      <div className="grid grid-cols-2 gap-2">
        <div>
          <Label>{t("properties.connections")}</Label>
          <Select value={String(conns ?? item.connections)} onChange={(e) => applyConnections(Number(e.target.value))}>
            {CONN_OPTIONS.map((n) => (
              <option key={n} value={n}>{n === 0 ? t("newDownload.auto") : n}</option>
            ))}
          </Select>
        </div>
        <div>
          <Label>{t("properties.speedLimit")}</Label>
          <Select value={rate} onChange={(e) => applyRate(e.target.value)}>
            <option value="0">{t("rateLimit.unlimited")}</option>
            <option value={String(256 * 1024)}>256 KB/s</option>
            <option value={String(1024 * 1024)}>1 MB/s</option>
            <option value={String(5 * 1024 * 1024)}>5 MB/s</option>
          </Select>
        </div>
      </div>
      <div className="min-h-0">
        <Label>{t("properties.progressMap")}</Label>
        <ProgressMap parts={item.parts} connections={conns ?? item.connections} />
      </div>
      <div>
        <Label>{t("properties.url")}</Label>
        <div className="flex items-center gap-2">
          <div className="min-w-0 flex-1 truncate text-[12px] text-muted-foreground" title={item.url}>{item.url}</div>
          <Button size="sm" disabled={controls.busy} onClick={handleCopyUrl}>{urlCopied ? "OK" : t("downloadRow.copyUrl")}</Button>
        </div>
      </div>
      {showRefresh && (
        <div>
          <Label>{t("properties.refreshUrl")}</Label>
          <div className="flex gap-1.5">
            <Input value={newUrl} onChange={(e) => setNewUrl(e.target.value)} placeholder={t("properties.newUrl")} />
            <Button disabled={controls.busy || !newUrl.trim()} onClick={refresh}>{t("properties.apply")}</Button>
          </div>
        </div>
      )}
    </div>
  );
}
