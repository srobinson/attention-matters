"use client";

import { useState } from "react";
import { ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { amRetrieve } from "@/lib/am-client";
import { loadSettings } from "@/lib/settings";
import type { QueryIndexEntry } from "@/lib/types";
import { FeedbackButtons } from "./feedback-buttons";
import { getCategoryColor } from "./shared";

interface SearchResultProps {
  entry: QueryIndexEntry;
  query: string;
  maxScore: number;
}

function scoreToStrength(score: number): { label: string; color: string } {
  if (score >= 7) return { label: "Strong match", color: "var(--color-conscious)" };
  if (score >= 4) return { label: "Moderate match", color: "var(--color-subconscious)" };
  return { label: "Weak match", color: "var(--color-text-secondary)" };
}

/**
 * Single search result with expandable full text.
 * Shows category badge, match strength bar, summary.
 * Clicking expands to load full content via retrieve endpoint.
 */
export function SearchResult({ entry, query, maxScore }: SearchResultProps) {
  const [expanded, setExpanded] = useState(false);
  const strength = scoreToStrength(entry.score);

  // Phase 2: fetch full content only when expanded
  const { data: fullText, isLoading: textLoading } = useQuery({
    queryKey: ["am", "retrieve", entry.id],
    queryFn: async () => {
      const apiKey = loadSettings().apiKey || undefined;
      const result = await amRetrieve({ ids: [entry.id] }, apiKey);
      return result.entries?.[0]?.text ?? "";
    },
    enabled: expanded,
  });

  // Relative strength bar width (normalized to max score in results)
  const barWidth = maxScore > 0 ? (entry.score / maxScore) * 100 : 0;

  return (
    <div
      className="mx-2 rounded-md border"
      style={{
        borderColor: "var(--color-border-subtle)",
        background: "var(--color-surface)",
      }}
    >
      <button
        className="flex w-full items-start gap-2 px-3 py-2 text-left"
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
        aria-controls={`search-${entry.id}`}
      >
        <div className="flex flex-1 flex-col gap-1.5 min-w-0">
          {/* Category + type badges */}
          <div className="flex items-center gap-2 flex-wrap">
            <span
              className="inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider"
              style={{
                background: `${getCategoryColor(entry.category)}20`,
                color: getCategoryColor(entry.category),
              }}
            >
              {entry.type}
            </span>
            <span
              className="text-[10px] font-medium"
              style={{ color: strength.color }}
            >
              {strength.label}
            </span>
            <span
              className="text-[10px] tabular-nums"
              style={{ color: "var(--color-text-secondary)" }}
            >
              Gen {entry.epoch}
            </span>
            <span
              className="text-[10px] tabular-nums"
              style={{ color: "var(--color-text-secondary)" }}
            >
              ~{entry.token_estimate} tokens
            </span>
          </div>

          {/* Strength bar */}
          <div
            className="h-1 w-full rounded-full"
            style={{ background: "var(--color-surface-raised)" }}
          >
            <div
              className="h-1 rounded-full transition-all"
              style={{
                width: `${barWidth}%`,
                background: strength.color,
              }}
            />
          </div>

          {/* Summary */}
          <p
            className="text-xs leading-relaxed"
            style={{ color: "var(--color-text-primary)" }}
          >
            {entry.summary.length > 150
              ? entry.summary.slice(0, 150).trimEnd() + "..."
              : entry.summary}
          </p>
        </div>

        <div
          className="mt-0.5 flex-shrink-0"
          style={{ color: "var(--color-text-secondary)" }}
        >
          {expanded ? (
            <ChevronDown className="h-3.5 w-3.5" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5" />
          )}
        </div>
      </button>

      {/* Expanded: full text + feedback */}
      {expanded && (
        <div
          id={`search-${entry.id}`}
          className="border-t px-3 py-2"
          style={{ borderColor: "var(--color-border-subtle)" }}
        >
          {textLoading && (
            <div className="flex items-center gap-2 py-2">
              <Loader2
                className="h-3 w-3 animate-spin"
                style={{ color: "var(--color-text-secondary)" }}
              />
              <span
                className="text-[11px]"
                style={{ color: "var(--color-text-secondary)" }}
              >
                Loading full text...
              </span>
            </div>
          )}

          {!textLoading && fullText !== undefined && (
            <pre
              className="mb-2 whitespace-pre-wrap text-xs leading-relaxed"
              style={{ color: "var(--color-text-primary)" }}
            >
              {fullText}
            </pre>
          )}

          <FeedbackButtons
            query={query}
            neighborhoodIds={[entry.id]}
          />
        </div>
      )}
    </div>
  );
}
