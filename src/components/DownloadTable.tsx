import { memo, useCallback, useRef } from "react";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { useDownloads } from "../query/downloadQueries";
import { useDownloadSpeed } from "../hooks/useDownloadSpeed";
import FileIcon from "./FileIcon";
import { t } from "../i18n";
import { applyFilter, openFile, openFolder } from "../utils/download";
import { useAppContext } from "../contexts/AppContext";
import { useContextMenu } from "../hooks/useContextMenu";
import { Checkbox } from "./ui/checkbox";
import { overallPercent } from "../utils/progressMap";
import { formatBytes, statusString, statusLabel, isFailed, isActiveStatus, failureText } from "../utils/format";
import { computeETA } from "../hooks/useDownloadSpeed";
import type { DownloadItem } from "../types";
import type { StatusFilter, TypeFilter } from "../utils/url";
import { Progress } from "./ui/progress";

interface DownloadTableProps {
  filter: StatusFilter;
  query?: string;
  typeFilter?: TypeFilter;
}

const menuItem = "cursor-pointer rounded-sm px-3 py-1.5 text-[13px] outline-none data-[highlighted]:bg-muted";

function isSegmentItem(item: DownloadItem): boolean {
  const ct = (item.content_type || "").toLowerCase();
  return ct.includes("mpegurl") || item.file_name.toLowerCase().endsWith(".m3u8");
}

function StatusCell({ item }: { item: DownloadItem }) {
  const s = statusString(item.status);
  const pct = overallPercent(item.downloaded, item.total_size, item.status);
  const live = isActiveStatus(item.status);
  if (s === "connecting") return <span>{t("status.connecting")}</span>;
  if (s === "retrying" || s === "merging") {
    const label = s === "retrying" ? t("status.retrying") : t("status.merging");
    return <span>{item.total_size > 0 ? `${label} · ${pct}%` : label}</span>;
  }
  if (isFailed(item.status) || s === "failed") {
    const { code, message } = failureText(item.status, item.error_message);
    const label = code != null ? `${t("status.failed")} · HTTP ${code}` : t("status.failed");
    return <span title={message || undefined}>{label}</span>;
  }
  if (live && item.total_size > 0) {
    return (
      <div className="flex items-center gap-2">
        <Progress className="w-20" value={pct} />
        <span className="tabular text-[12px]">{pct}%</span>
      </div>
    );
  }
  return <span>{statusLabel(item.status)}</span>;
}

export default function DownloadTable({ filter, query = "", typeFilter = "all" }: DownloadTableProps) {
  const { selectedIds, selectionActions, actions } = useAppContext();
  const { onStop, onDelete, onProperties, onRedownload, onResume } = actions;
  const { data: downloads = [], isLoading } = useDownloads();
  const filtered = applyFilter(downloads, filter, query, typeFilter);
  const speeds = useDownloadSpeed(filtered);
  const { menuState, menuRef, handleContext, closeMenu } = useContextMenu();
  const anchor = useRef<number | null>(null);

  const selectAllChecked = filtered.length > 0 && filtered.every((d) => selectedIds.has(d.id));
  const selectAllIndeterminate = !selectAllChecked && filtered.some((d) => selectedIds.has(d.id));

  const toggleSelectAll = () => {
    if (selectAllChecked) selectionActions.clearSelection();
    else selectionActions.select(new Set(filtered.map((d) => d.id)));
  };

  const onRowClick = (e: React.MouseEvent, id: number) => {
    if ((e.target as HTMLElement).closest("[data-row-check]")) return;
    const ids = filtered.map((d) => d.id);
    if (e.shiftKey && anchor.current != null) {
      const a = ids.indexOf(anchor.current);
      const b = ids.indexOf(id);
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        const next = new Set<number>();
        if (e.metaKey || e.ctrlKey) selectedIds.forEach((x) => next.add(x));
        for (let i = lo; i <= hi; i++) next.add(ids[i]!);
        selectionActions.select(next);
        return;
      }
    }
    if (e.metaKey || e.ctrlKey) selectionActions.toggle(id);
    else selectionActions.select(new Set([id]));
    anchor.current = id;
  };

  const onDoubleClick = useCallback(async (item: DownloadItem) => {
    if (item.status === "completed") await openFile(item.save_path);
    else actions.onProperties(item.id);
  }, [actions]);

  if (isLoading) {
    return <div className="p-6 text-muted-foreground">{t("downloadTable.loading")}</div>;
  }
  if (filtered.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-muted-foreground">
        {downloads.length === 0 ? t("downloadTable.empty") : t("downloadTable.noMatch")}
      </div>
    );
  }

  const menuItemFor = filtered.find((d) => d.id === menuState?.id) ?? null;

  return (
    <div className="relative">
      <table className="w-full border-collapse text-[13px]">
        <thead className="sticky top-0 z-10 bg-muted text-left text-[11px] uppercase tracking-wide text-muted-foreground">
          <tr>
            <th className="w-8 px-2 py-1">
              <Checkbox checked={selectAllChecked} onCheckedChange={toggleSelectAll} {...(selectAllIndeterminate ? { "data-state": "indeterminate" } : {})} />
            </th>
            <th className="px-2 py-1">{t("downloadTable.fileName")}</th>
            <th className="w-[140px] px-2 py-1">{t("downloadTable.size")}</th>
            <th className="w-[180px] px-2 py-1">{t("downloadTable.status")}</th>
            <th className="w-[90px] px-2 py-1">{t("downloadTable.speed")}</th>
            <th className="w-[90px] px-2 py-1">{t("downloadTable.remain")}</th>
            <th className="w-[90px] px-2 py-1">{t("downloadTable.proxy")}</th>
          </tr>
        </thead>
        <tbody>
          {filtered.map((row) => (
            <DownloadRow
              key={row.id}
              item={row}
              selected={selectedIds.has(row.id)}
              speed={speeds.get(row.id)?.display ?? "—"}
              bps={speeds.get(row.id)?.bps ?? 0}
              onToggle={() => selectionActions.toggle(row.id)}
              onClick={(e) => onRowClick(e, row.id)}
              onContext={(e) => handleContext(e, row.id)}
              onDoubleClick={() => onDoubleClick(row)}
            />
          ))}
        </tbody>
      </table>
      {menuState && menuItemFor && (
        <DropdownMenu.Root open onOpenChange={(open) => { if (!open) closeMenu(); }}>
          <DropdownMenu.Trigger asChild>
            <button
              type="button"
              aria-hidden
              tabIndex={-1}
              className="fixed h-px w-px p-0 opacity-0"
              style={{ left: menuState.x, top: menuState.y }}
            />
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content
              ref={menuRef}
              className="z-50 min-w-[168px] rounded-md border border-border bg-card py-1 shadow-sm"
              align="start"
              sideOffset={2}
            >
              <RowMenu
                item={menuItemFor}
                onResume={() => onResume(menuItemFor.id)}
                onPause={() => onStop(menuItemFor.id)}
                onOpen={() => { void openFile(menuItemFor.save_path); }}
                onOpenFolder={() => { void openFolder(menuItemFor.save_path); }}
                onRedownload={() => { void onRedownload(menuItemFor); }}
                onCopy={() => { void navigator.clipboard.writeText(menuItemFor.url); }}
                onDetails={() => onProperties(menuItemFor.id)}
                onDelete={() => onDelete([menuItemFor.id])}
              />
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      )}
    </div>
  );
}

function RowMenu({
  item, onResume, onPause, onOpen, onOpenFolder, onRedownload, onCopy, onDetails, onDelete,
}: {
  item: DownloadItem;
  onResume: () => void;
  onPause: () => void;
  onOpen: () => void;
  onOpenFolder: () => void;
  onRedownload: () => void;
  onCopy: () => void;
  onDetails: () => void;
  onDelete: () => void;
}) {
  const s = statusString(item.status);
  return (
    <>
      {s === "paused" && <DropdownMenu.Item className={menuItem} onSelect={onResume}>{t("downloadRow.resume")}</DropdownMenu.Item>}
      {isActiveStatus(item.status) && <DropdownMenu.Item className={menuItem} onSelect={onPause}>{t("downloadRow.pause")}</DropdownMenu.Item>}
      {item.status === "completed" && <DropdownMenu.Item className={menuItem} onSelect={onOpen}>{t("downloadRow.open")}</DropdownMenu.Item>}
      <DropdownMenu.Item className={menuItem} onSelect={onOpenFolder}>{t("downloadRow.openFolder")}</DropdownMenu.Item>
      <DropdownMenu.Item className={menuItem} onSelect={onRedownload}>{t("toolbar.redownload")}</DropdownMenu.Item>
      <DropdownMenu.Item className={menuItem} onSelect={onCopy}>{t("downloadRow.copyUrl")}</DropdownMenu.Item>
      <DropdownMenu.Item className={menuItem} onSelect={onDetails}>{t("downloadRow.details")}</DropdownMenu.Item>
      <DropdownMenu.Separator className="my-1 h-px bg-border" />
      <DropdownMenu.Item className={`${menuItem} text-destructive`} onSelect={onDelete}>{t("toolbar.delete")}</DropdownMenu.Item>
    </>
  );
}

const DownloadRow = memo(function DownloadRow({
  item, selected, speed, bps, onToggle, onClick, onContext, onDoubleClick,
}: {
  item: DownloadItem;
  selected: boolean;
  speed: string;
  bps: number;
  onToggle: () => void;
  onClick: (e: React.MouseEvent) => void;
  onContext: (e: React.MouseEvent) => void;
  onDoubleClick: () => void;
}) {
  const live = isActiveStatus(item.status);
  const size = isSegmentItem(item)
    ? (item.total_size > 0
      ? t("properties.segments").replace("{done}", String(item.downloaded)).replace("{total}", String(item.total_size))
      : "—")
    : item.total_size === 0
      ? "—"
      : item.status === "completed"
        ? formatBytes(item.total_size)
        : `${formatBytes(item.downloaded)} / ${formatBytes(item.total_size)}`;
  return (
    <tr
      className={`border-b border-border hover:bg-muted/70 ${selected ? "bg-muted" : ""} ${live ? "row-live" : ""}`}
      onClick={onClick}
      onContextMenu={onContext}
      onDoubleClick={onDoubleClick}
    >
      <td className="px-2 py-1" data-row-check onClick={(e) => e.stopPropagation()}>
        <Checkbox checked={selected} onCheckedChange={onToggle} />
      </td>
      <td className="max-w-0 px-2 py-1">
        <div className="flex min-w-0 items-center gap-2">
          <FileIcon
            id={item.id}
            fileName={item.file_name}
            path={item.save_path}
            mimeType={item.content_type}
            completed={item.status === "completed"}
            size={18}
          />
          <span className="truncate font-medium">{item.file_name}</span>
        </div>
      </td>
      <td className="tabular px-2 py-1 whitespace-nowrap">{size}</td>
      <td className="px-2 py-1"><StatusCell item={item} /></td>
      <td className="tabular px-2 py-1 whitespace-nowrap">{live ? speed : "—"}</td>
      <td className="tabular px-2 py-1 whitespace-nowrap">{live ? computeETA(item, bps) : "—"}</td>
      <td className="truncate px-2 py-1 text-muted-foreground">{item.proxy_name || "—"}</td>
    </tr>
  );
});
