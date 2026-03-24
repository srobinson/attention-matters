"use client";

import { useQuery } from "@tanstack/react-query";
import { amStats } from "@/lib/am-client";
import { loadSettings } from "@/lib/settings";
import type { StatsResponse } from "@/lib/types";

/**
 * Format byte count into human-readable size.
 */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

interface StatItemProps {
  label: string;
  value: string | number;
  tooltip: string;
}

function StatItem({ label, value, tooltip }: StatItemProps) {
  return (
    <div
      className="flex flex-col gap-0.5"
      title={tooltip}
    >
      <span
        className="text-[11px] font-medium tabular-nums"
        style={{ color: "var(--color-text-primary)" }}
      >
        {value}
      </span>
      <span
        className="text-[10px]"
        style={{ color: "var(--color-text-secondary)" }}
      >
        {label}
      </span>
    </div>
  );
}

/**
 * Compact stats header for the sidebar showing live memory system metrics.
 * Fetches from GET /api/am/stats via TanStack Query.
 * User-facing terminology: "pinned" (not "conscious"), "times recalled" (not "activation").
 */
export function StatsHeader() {
  const { data, isLoading, isFetching } = useQuery<StatsResponse>({
    queryKey: ["am", "stats"],
    queryFn: () => amStats(loadSettings().apiKey || undefined),
    refetchInterval: 60_000,
  });

  if (isLoading) {
    return (
      <div
        className="flex items-center gap-2 px-3 py-2"
        style={{ borderColor: "var(--color-border)" }}
      >
        <div
          className="h-3 w-16 animate-pulse rounded"
          style={{ background: "var(--color-surface-raised)" }}
        />
        <div
          className="h-3 w-12 animate-pulse rounded"
          style={{ background: "var(--color-surface-raised)" }}
        />
      </div>
    );
  }

  const n = data?.n ?? 0;
  const episodes = data?.episodes ?? 0;
  const pinned = data?.conscious ?? 0;
  const dbSize = data?.db_size_bytes;
  const activation = data?.activation;

  return (
    <div className="relative flex flex-wrap gap-x-4 gap-y-1 px-3 py-2">
      {/* Stale indicator */}
      {isFetching && (
        <div
          className="absolute right-2 top-2 h-1.5 w-1.5 animate-pulse rounded-full"
          style={{ background: "var(--color-salient)" }}
          title="Refreshing stats..."
        />
      )}

      <StatItem
        label="memories"
        value={n.toLocaleString()}
        tooltip={n === 0
          ? "Total word occurrences stored in the memory system"
          : `${n.toLocaleString()} word occurrences across all episodes`}
      />

      <StatItem
        label="episodes"
        value={episodes.toLocaleString()}
        tooltip={episodes === 0
          ? "Documents and conversations ingested into memory"
          : `${episodes.toLocaleString()} ingested documents and conversations`}
      />

      <StatItem
        label="pinned"
        value={pinned.toLocaleString()}
        tooltip={pinned === 0
          ? "Memories explicitly marked as important"
          : `${pinned.toLocaleString()} memories marked as important`}
      />

      {dbSize !== undefined && (
        <StatItem
          label="db size"
          value={formatBytes(dbSize)}
          tooltip={`Database file size: ${dbSize.toLocaleString()} bytes`}
        />
      )}

      {activation && (
        <StatItem
          label="recall avg"
          value={activation.mean.toFixed(1)}
          tooltip={`Times recalled: avg ${activation.mean.toFixed(1)}, max ${activation.max}, ${activation.zero_count.toLocaleString()} never recalled`}
        />
      )}
    </div>
  );
}
