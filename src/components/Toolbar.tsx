import { Plus, Play, Square, Trash2, Settings, Gauge, MoreHorizontal, ScrollText, Globe, Info, LogOut } from "lucide-react";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { t } from "../i18n";
import { useAppContext } from "../contexts/AppContext";
import { Button } from "./ui/button";
import { Select } from "./ui/select";
import { useSettings } from "../query/downloadQueries";
import { tauriClient } from "../tauriClient";

interface ToolbarProps {
  hasDownloadingSelected: boolean;
  hasPausedSelected: boolean;
  hasDeletable: boolean;
}

const RATE_OPTIONS = [
  { v: 0, label: "∞" },
  { v: 256 * 1024, label: "256 KB/s" },
  { v: 1024 * 1024, label: "1 MB/s" },
  { v: 5 * 1024 * 1024, label: "5 MB/s" },
  { v: 10 * 1024 * 1024, label: "10 MB/s" },
];

const menuItem = "cursor-pointer rounded-sm px-3 py-1.5 text-[13px] outline-none data-[highlighted]:bg-muted";

export default function Toolbar({
  hasDownloadingSelected, hasPausedSelected, hasDeletable,
}: ToolbarProps) {
  const { actions } = useAppContext();
  const { onNewDownload, onExtension, onSettings, onAbout, onQuit, onLog,
    onResumeSelected, onPauseSelected, onDeleteSelected } = actions;
  const { settings, saveSettings } = useSettings();

  const onGlobalRate = async (value: string) => {
    const bps = Number(value);
    await tauriClient.setGlobalRateLimit(bps);
    if (settings) {
      await saveSettings({ ...settings, global_rate_limit: bps });
    }
  };

  return (
    <div className="flex items-center gap-1.5 overflow-x-auto border-b border-border bg-muted px-2 py-1.5 whitespace-nowrap">
      <Button variant="default" onClick={onNewDownload}>
        <Plus className="h-4 w-4" /> {t("toolbar.new")}
      </Button>
      <Button onClick={onResumeSelected} disabled={!hasPausedSelected}>
        <Play className="h-4 w-4" /> {t("toolbar.resume")}
      </Button>
      <Button onClick={onPauseSelected} disabled={!hasDownloadingSelected}>
        <Square className="h-4 w-4" /> {t("downloadRow.pause")}
      </Button>
      <Button variant="destructive" onClick={onDeleteSelected} disabled={!hasDeletable}>
        <Trash2 className="h-4 w-4" /> {t("toolbar.delete")}
      </Button>
      <div className="ml-2 flex items-center gap-1.5 text-[13px] text-muted-foreground">
        <Gauge className="h-4 w-4" />
        <Select
          className="w-[120px]"
          value={String(settings?.global_rate_limit ?? 0)}
          onChange={(e) => onGlobalRate(e.target.value)}
        >
          {RATE_OPTIONS.map((o) => (
            <option key={o.v} value={o.v}>{o.label}</option>
          ))}
        </Select>
      </div>
      <div className="flex-1" />
      <Button variant="ghost" size="icon" onClick={onSettings} title={t("toolbar.settings")}>
        <Settings className="h-4 w-4" />
      </Button>
      <DropdownMenu.Root>
        <DropdownMenu.Trigger asChild>
          <Button variant="ghost" size="icon" title={t("toolbar.more")}>
            <MoreHorizontal className="h-4 w-4" />
          </Button>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content
            className="z-50 min-w-[160px] rounded-md border border-border bg-card py-1 shadow-sm"
            align="end"
            sideOffset={4}
          >
            <DropdownMenu.Item className={menuItem} onSelect={onExtension}>
              <span className="inline-flex items-center gap-2"><Globe className="h-3.5 w-3.5" /> {t("toolbar.extension")}</span>
            </DropdownMenu.Item>
            <DropdownMenu.Item className={menuItem} onSelect={onLog}>
              <span className="inline-flex items-center gap-2"><ScrollText className="h-3.5 w-3.5" /> {t("toolbar.log")}</span>
            </DropdownMenu.Item>
            <DropdownMenu.Item className={menuItem} onSelect={onAbout}>
              <span className="inline-flex items-center gap-2"><Info className="h-3.5 w-3.5" /> {t("toolbar.about")}</span>
            </DropdownMenu.Item>
            <DropdownMenu.Separator className="my-1 h-px bg-border" />
            <DropdownMenu.Item className={menuItem} onSelect={() => { void onQuit(); }}>
              <span className="inline-flex items-center gap-2"><LogOut className="h-3.5 w-3.5" /> {t("toolbar.quit")}</span>
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
    </div>
  );
}
