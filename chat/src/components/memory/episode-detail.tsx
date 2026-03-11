"use client";

import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, Loader2 } from "lucide-react";
import { amEpisodeNeighborhoods } from "@/lib/am-client";
import { loadSettings } from "@/lib/settings";
import type { Episode, EpisodeNeighborhood } from "@/lib/types";
import { NeighborhoodList } from "./neighborhood-list";
import type { NeighborhoodEntry } from "./neighborhood-list";

interface EpisodeDetailProps {
  episode: Episode;
  onBack: () => void;
}

/**
 * Map backend EpisodeNeighborhood to the NeighborhoodEntry shape
 * consumed by NeighborhoodList.
 */
function toNeighborhoodEntry(n: EpisodeNeighborhood): NeighborhoodEntry {
  return {
    id: n.id,
    category: n.is_conscious ? "Conscious" : "Subconscious",
    type: n.type,
    episode: n.episode,
    tokens: n.tokens,
    text: n.text,
  };
}

/**
 * Episode detail view showing metadata and topic clusters.
 * Opens when clicking an episode in the sidebar list.
 * Fetches neighborhoods via the dedicated
 * GET /api/am/episodes/:id/neighborhoods endpoint.
 *
 * User-facing terminology:
 *   neighborhood -> topic cluster
 *   epoch -> generation
 *   activation count -> times recalled
 */
export function EpisodeDetail({ episode, onBack }: EpisodeDetailProps) {
  const { data: neighborhoods, isLoading } = useQuery({
    queryKey: ["am", "episode-detail", episode.id],
    queryFn: async (): Promise<NeighborhoodEntry[]> => {
      const apiKey = loadSettings().apiKey || undefined;
      const raw = await amEpisodeNeighborhoods(episode.id, apiKey);
      return raw.map(toNeighborhoodEntry);
    },
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
