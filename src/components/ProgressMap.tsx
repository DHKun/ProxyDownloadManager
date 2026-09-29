import type { DownloadPart, DownloadStatus } from "../types";
import { t } from "../i18n";

const ROW_H = 28;
const VISIBLE = 5;

export interface ThreadBar {
  index: number;
  percent: number;
}

/** One bar per connection, folding parts onto connection slots. */
export function threadBars(
  parts: DownloadPart[],
  connections: number,
  status?: DownloadStatus,
): ThreadBar[] {
  if (connections <= 0 && parts.length === 0) return [];
  const n = Math.max(1, connections || parts.length || 1);
  if (status === "completed") {
    return Array.from({ length: n }, (_, i) => ({ index: i + 1, percent: 100 }));
  }
  const buckets = Array.from({ length: n }, () => ({ downloaded: 0, size: 0 }));
  if (parts.length === 0) {
    return buckets.map((_, i) => ({ index: i + 1, percent: 0 }));
  }
  for (const p of parts) {
    const i = (p.index % n + n) % n;
    buckets[i].downloaded += p.downloaded;
    buckets[i].size += Math.max(0, p.end - p.start);
  }
  return buckets.map((b, i) => ({
    index: i + 1,
    percent: b.size > 0 ? Math.min(100, Math.floor((b.downloaded / b.size) * 100)) : 0,
  }));
}

interface ThreadBarsProps {
  parts: DownloadPart[];
  connections: number;
  status?: DownloadStatus;
}

export default function ProgressMap({ parts, connections, status }: ThreadBarsProps) {
  const bars = threadBars(parts, connections, status);
  if (!bars.length) {
    return <div className="text-[13px] text-muted-foreground">{t("properties.progressMapEmpty")}</div>;
  }
  return (
    <div
      className="overflow-y-auto pr-1"
      style={{ maxHeight: ROW_H * VISIBLE }}
    >
      {bars.map((bar) => (
        <div key={bar.index} className="flex items-center gap-2" style={{ height: ROW_H }}>
          <span className="w-8 shrink-0 tabular text-[12px] text-muted-foreground">#{bar.index}</span>
          <div className="h-2.5 min-w-0 flex-1 overflow-hidden rounded-sm bg-muted">
            <div className="h-full bg-foreground" style={{ width: `${bar.percent}%` }} />
          </div>
          <span className="w-10 shrink-0 tabular text-right text-[12px]">{bar.percent}%</span>
        </div>
      ))}
    </div>
  );
}
