"use client";

import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, Loader2 } from "lucide-react";
import { amQueryIndex, amRetrieve } from "@/lib/am-client";
import { loadSettings } from "@/lib/settings";
import type { Episode } from "@/lib/types";
import { NeighborhoodList } from "./neighborhood-list";
import type { NeighborhoodEntry } from "./neighborhood-list";

interface EpisodeDetailProps {
  episode: Episode;
  onBack: () => void;
}

/**
 * Fetch topic clusters related to an episode using two-phase retrieval:
 * 1. query_index with episode name to find related neighborhood IDs
 * 2. retrieve with those IDs to get full text
 */
async function fetchEpisodeNeighborhoods(
  episodeName: string
): Promise<NeighborhoodEntry[]> {
  const apiKey = loadSettings().apiKey || undefined;

  // Phase 1: query index by episode name
  const indexResult = await amQueryIndex({ text: episodeName }, apiKey);

  // The index result has entries with IDs
  interface IndexEntry {
    id: string;
    score: number;
    summary: string;
    epoch: number;
    type: string;
  }

  const indexData = indexResult as { entries?: IndexEntry[] };
  const entries = indexData.entries ?? [];

  if (entries.length === 0) return [];

  const ids = entries.map((e: IndexEntry) => e.id);

  // Phase 2: retrieve full content
  const retrieveResult = await amRetrieve({ ids }, apiKey);

  interface RetrieveEntry {
    id: string;
    category: string;
    type: string;
    episode: string;
    tokens: number;
    text: string;
  }

  const retrieveData = retrieveResult as { entries?: RetrieveEntry[] };
  const neighborhoods: NeighborhoodEntry[] = (retrieveData.entries ?? []).map(
    (e: RetrieveEntry) => ({
      id: e.id,
      category: e.category,
      type: e.type,
      episode: e.episode,
      tokens: e.tokens,
      text: e.text,
    })
  );

  return neighborhoods;
}

/**
 * Episode detail view showing metadata and topic clusters.
 * Opens when clicking an episode in the sidebar list.
 * Uses two-phase retrieval (query_index then retrieve) to
 * load neighborhood data since the episodes endpoint is lightweight.
 *
 * User-facing terminology:
 *   neighborhood -> topic cluster
 *   epoch -> generation
 *   activation count -> times recalled
 */
export function EpisodeDetail({ episode, onBack }: EpisodeDetailProps) {
  const { data: neighborhoods, isLoading } = useQuery({
    queryKey: ["am", "episode-detail", episode.name],
    queryFn: () => fetchEpisodeNeighborhoods(episode.name),
  });

  const date = formatTimestamp(episode.created);

  return (
    <div className="flex h-full flex-col">
      {/* Header with back button */}
      <div
        className="flex flex-shrink-0 items-center gap-2 border-b px-3 py-2"
        style={{ borderColor: "var(--color-border)" }}
      >
        <button
          onClick={onBack}
          className="flex h-6 w-6 items-center justify-center rounded transition-colors hover:opacity-80"
          style={{ color: "var(--color-text-secondary)" }}
          aria-label="Back to episode list"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
        </button>
        <div className="min-w-0 flex-1">
          <h3
            className="truncate text-xs font-medium"
            style={{ color: "var(--color-text-primary)" }}
          >
            {episode.name}
          </h3>
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
              {episode.total_occurrences} occurrences
            </span>
            <span
              className="text-[10px]"
              style={{ color: "var(--color-text-secondary)" }}
            >
              {date}
            </span>
          </div>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto py-2">
        {isLoading && (
          <div className="flex items-center justify-center py-8">
            <Loader2
              className="h-4 w-4 animate-spin"
              style={{ color: "var(--color-text-secondary)" }}
            />
          </div>
        )}

        {!isLoading && neighborhoods && (
          <NeighborhoodList
            neighborhoods={neighborhoods}
            query={episode.name}
          />
        )}
      </div>
    </div>
  );
}

function formatTimestamp(isoString: string): string {
  try {
    return new Date(isoString).toLocaleDateString("en-US", {
      month: "short",
      day: "numeric",
      year: "numeric",
    });
  } catch {
    return "";
  }
}
