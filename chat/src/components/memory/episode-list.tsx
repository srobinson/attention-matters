"use client";

import { useState, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Search, Loader2, Upload } from "lucide-react";
import { amEpisodes } from "@/lib/am-client";
import { loadSettings } from "@/lib/settings";
import type { Episode } from "@/lib/types";

interface EpisodeListProps {
  onSelectEpisode?: (episode: Episode) => void;
  onUploadClick?: () => void;
}

/**
 * Episode list with client-side search filter.
 * Fetches from GET /api/am/episodes via TanStack Query.
 * User-facing terminology: "topic cluster" (not "neighborhood").
 */
export function EpisodeList({ onSelectEpisode, onUploadClick }: EpisodeListProps) {
  const [filter, setFilter] = useState("");

  const { data: episodes, isLoading, error } = useQuery<Episode[]>({
    queryKey: ["am", "episodes"],
    queryFn: () => amEpisodes(loadSettings().apiKey || undefined),
  });

  const filtered = useMemo(() => {
    if (!episodes) return [];
    if (!filter.trim()) return episodes;
    const term = filter.toLowerCase();
    return episodes.filter((ep) =>
      ep.name.toLowerCase().includes(term)
    );
  }, [episodes, filter]);

  return (
    <div className="flex h-full flex-col">
      {/* Search filter */}
      <div className="flex-shrink-0 px-2 pb-2">
        <div
          className="flex items-center gap-1.5 rounded border px-2 py-1"
          style={{
            borderColor: "var(--color-border)",
            background: "var(--color-surface-raised)",
          }}
        >
          <Search
            className="h-3 w-3 flex-shrink-0"
            style={{ color: "var(--color-text-secondary)" }}
          />
          <input
            type="text"
            placeholder="Filter episodes..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            className="w-full bg-transparent text-xs outline-none placeholder:opacity-50"
            style={{ color: "var(--color-text-primary)" }}
            aria-label="Filter episodes by name"
          />
        </div>
      </div>

      {/* List */}
      <div className="flex-1 overflow-y-auto px-2">
        {isLoading && (
          <div className="flex items-center justify-center py-8">
            <Loader2
              className="h-4 w-4 animate-spin"
              style={{ color: "var(--color-text-secondary)" }}
            />
          </div>
        )}

        {error && (
          <p
            className="px-1 py-4 text-center text-xs"
            style={{ color: "#ef4444" }}
          >
            Failed to load episodes
          </p>
        )}

        {!isLoading && !error && filtered.length === 0 && (
          <div className="flex flex-col items-center gap-3 px-2 py-8 text-center">
            <p
              className="text-xs leading-relaxed"
              style={{ color: "var(--color-text-secondary)" }}
            >
              {episodes && episodes.length === 0
                ? "Your memory is empty. Start a conversation or upload a document to build your memory."
                : "No episodes match your filter."}
            </p>
            {episodes && episodes.length === 0 && onUploadClick && (
              <button
                onClick={onUploadClick}
                className="flex items-center gap-1.5 rounded border px-3 py-1.5 text-xs transition-colors hover:opacity-80"
                style={{
                  borderColor: "var(--color-salient)",
                  color: "var(--color-salient)",
                }}
              >
                <Upload className="h-3 w-3" />
                Upload a document
              </button>
            )}
          </div>
        )}

        {filtered.map((episode) => (
          <EpisodeItem
            key={episode.id}
            episode={episode}
            onClick={() => onSelectEpisode?.(episode)}
          />
        ))}
      </div>
    </div>
  );
}

function EpisodeItem({
  episode,
  onClick,
}: {
  episode: Episode;
  onClick: () => void;
}) {
  const date = formatDate(episode.created);

  return (
    <button
      onClick={onClick}
      className="mb-1 flex w-full flex-col gap-0.5 rounded-md px-2 py-1.5 text-left transition-colors hover:opacity-90"
      style={{
        background: "transparent",
        borderLeft: episode.is_conscious
          ? "3px solid var(--color-conscious)"
          : "3px solid transparent",
      }}
      onMouseEnter={(e) => {
        e.currentTarget.style.background = "var(--color-surface-raised)";
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.background = "transparent";
      }}
    >
      <span
        className="truncate text-xs font-medium"
        style={{ color: "var(--color-text-primary)" }}
      >
        {episode.name}
      </span>
      <div className="flex items-center gap-2">
        <span
          className="text-[10px]"
          style={{ color: "var(--color-text-secondary)" }}
        >
          {episode.neighborhood_count} topic clusters
        </span>
        <span
          className="text-[10px]"
          style={{ color: "var(--color-text-secondary)" }}
        >
          {date}
        </span>
      </div>
    </button>
  );
}

function formatDate(isoString: string): string {
  try {
    const d = new Date(isoString);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    const days = Math.floor(diff / (1000 * 60 * 60 * 24));

    if (days === 0) return "Today";
    if (days === 1) return "Yesterday";
    if (days < 7) return `${days}d ago`;
    return d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
  } catch {
    return "";
  }
}
