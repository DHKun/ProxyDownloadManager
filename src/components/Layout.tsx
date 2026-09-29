import { useState } from "react";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import Toolbar from "./Toolbar";
import DownloadTable from "./DownloadTable";
import { useDownloads } from "../query/downloadQueries";
import { isActiveStatus } from "../utils/format";
import { t } from "../i18n";
import { useAppContext } from "../contexts/AppContext";
import { Input } from "./ui/input";
import { Button } from "./ui/button";
import type { TypeFilter } from "../utils/url";
import { cn } from "../lib/utils";

interface LayoutProps {
  className?: string;
}

export default function Layout({ className }: LayoutProps) {
  const { selectedIds, filter, setFilter } = useAppContext();
  const { data: downloads = [] } = useDownloads();
  const [query, setQuery] = useState("");
  const [type, setType] = useState<TypeFilter>("all");

  const selectedItems = downloads.filter((d) => selectedIds.has(d.id));
  const hasDownloadingSelected = selectedItems.some((d) => isActiveStatus(d.status));
  const hasPausedSelected = selectedItems.some((d) => d.status === "paused");
  const hasDeletable = selectedItems.length > 0;

  const counts = {
    all: downloads.length,
    downloading: downloads.filter((d) => isActiveStatus(d.status)).length,
    completed: downloads.filter((d) => d.status === "completed").length,
    incomplete: downloads.filter((d) => d.status !== "completed").length,
  };

  const typeOptions: Array<{ id: TypeFilter; label: string }> = [
    { id: "all", label: t("sidebar.typeAll") },
    { id: "archive", label: t("sidebar.typeArchive") },
    { id: "video", label: t("sidebar.typeVideo") },
    { id: "audio", label: t("sidebar.typeAudio") },
    { id: "document", label: t("sidebar.typeDocument") },
    { id: "other", label: t("sidebar.typeOther") },
  ];
  const typeLabels = Object.fromEntries(typeOptions.map((o) => [o.id, o.label])) as Record<TypeFilter, string>;

  const filters: Array<{ id: typeof filter; label: string; n: number }> = [
    { id: "all", label: t("sidebar.all"), n: counts.all },
    { id: "downloading", label: t("sidebar.downloading"), n: counts.downloading },
    { id: "completed", label: t("sidebar.completed"), n: counts.completed },
    { id: "incomplete", label: t("sidebar.incomplete"), n: counts.incomplete },
  ];

  return (
    <div className={cn("flex h-screen flex-col", className)}>
      <Toolbar
        hasDownloadingSelected={hasDownloadingSelected}
        hasPausedSelected={hasPausedSelected}
        hasDeletable={hasDeletable}
      />
      <div className="flex items-center gap-2 border-b border-border px-2 py-1.5">
        {filters.map((f) => (
          <button
            key={f.id}
            type="button"
            onClick={() => setFilter(f.id)}
            className={`h-8 rounded-md px-3 text-[13px] ${filter === f.id ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-muted"}`}
          >
            {f.label} {f.n}
          </button>
        ))}
        <div className="flex-1" />
        <Input
          className="w-[180px]"
          placeholder={t("sidebar.search")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <DropdownMenu.Root>
          <DropdownMenu.Trigger asChild>
            <Button variant="ghost">{t("sidebar.filter")}: {typeLabels[type]}</Button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content
              className="z-50 min-w-[140px] rounded-md border border-border bg-card py-1 shadow-sm"
              align="end"
              sideOffset={4}
            >
              {typeOptions.map((opt) => (
                <DropdownMenu.Item
                  key={opt.id}
                  className="cursor-pointer rounded-sm px-3 py-1.5 text-[13px] outline-none data-[highlighted]:bg-muted"
                  onSelect={() => setType(opt.id)}
                >
                  {opt.label}
                </DropdownMenu.Item>
              ))}
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <DownloadTable filter={filter} query={query} typeFilter={type} />
      </div>
    </div>
  );
}
