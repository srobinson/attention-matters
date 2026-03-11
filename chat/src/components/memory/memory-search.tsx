"use client";

import { useState, useCallback } from "react";
import { Search, Loader2 } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { amQueryIndex } from "@/lib/am-client";
import { loadSettings } from "@/lib/settings";
import type { QueryIndexEntry, QueryIndexResponse } from "@/lib/types";
import { SearchResult } from "./search-result";
import { getCategoryColor } from "./shared";

/**
 * Memory search panel for the sidebar Search tab.
 * Implements two-phase retrieval:
 *   Phase 1: query_index returns compact index entries
 *   Phase 2: clicking a result fetches full text via retrieve
 *
 * Results sorted by score descending, grouped by category.
 * Match strength displayed as Strong/Moderate/Weak labels with
 * relative bar visualization. No raw floating-point scores shown.
 */
export function MemorySearch() {
  const [query, setQuery] = useState("");
  const [submittedQuery, setSubmittedQuery] = useState("");

  const { data, isLoading, error } = useQuery<QueryIndexResponse>({
    queryKey: ["am", "query-index", submittedQuery],
    queryFn: () => {
      const apiKey = loadSettings().apiKey || undefined;
      return amQueryIndex({ text: submittedQuery }, apiKey);
    },
    enabled: submittedQuery.length > 0,
  });

  const handleSubmit = useCallback(
    (e: React.FormEvent<HTMLFormElement>) => {
      e.preventDefault();
      const trimmed = query.trim();
      if (trimmed.length > 0) {
        setSubmittedQuery(trimmed);
      }
    },
    [query]
  );

  const entries = data?.entries ?? [];
  const maxScore = entries.length > 0
    ? Math.max(...entries.map((e) => e.score))
    : 0;

  // Group by category, sorted by score within each group
  const grouped = groupByCategory(entries);
  const hasResults = submittedQuery.length > 0 && !isLoading && entries.length > 0;
  const noResults = submittedQuery.length > 0 && !isLoading && !error && entries.length === 0;

  return (
    <div className="flex h-full flex-col">
      {/* Search input */}
      <form onSubmit={handleSubmit} className="flex-shrink-0 px-2 pb-2">
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
            placeholder="Search your memory..."
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="w-full bg-transparent text-xs outline-none placeholder:opacity-50"
            style={{ color: "var(--color-text-primary)" }}
            aria-label="Search memory query"
          />
        </div>
      </form>

      {/* Results area */}
      <div className="flex-1 overflow-y-auto">
        {/* Initial state */}
        {submittedQuery.length === 0 && (
          <div className="flex flex-col items-center gap-2 px-3 py-8">
            <Search
              className="h-6 w-6"
              style={{ color: "var(--color-text-secondary)", opacity: 0.5 }}
            />
            <p
              className="text-center text-xs"
              style={{ color: "var(--color-text-secondary)" }}
            >
              Search your memory
            </p>
          </div>
        )}

        {/* Loading */}
        {isLoading && (
          <div className="flex items-center justify-center py-8">
            <Loader2
              className="h-4 w-4 animate-spin"
              style={{ color: "var(--color-text-secondary)" }}
            />
          </div>
        )}

        {/* Error */}
        {error && (
          <p
            className="px-3 py-4 text-center text-xs"
            style={{ color: "var(--color-error)" }}
          >
            Search failed. Try again.
          </p>
        )}

        {/* No results */}
        {noResults && (
          <p
            className="px-3 py-4 text-center text-xs"
            style={{ color: "var(--color-text-secondary)" }}
          >
            No matches found. Try a broader query.
          </p>
        )}

        {/* Results */}
        {hasResults && (
          <div className="flex flex-col gap-1">
            {/* Summary stats */}
            <div className="flex items-center gap-2 px-3 pb-1">
              <span
                className="text-[10px] tabular-nums"
                style={{ color: "var(--color-text-secondary)" }}
              >
                {entries.length} results from {data?.total_candidates ?? 0} candidates
              </span>
            </div>

            {/* Grouped results */}
            {grouped.map(([category, categoryEntries]) => (
              <div key={category} className="mb-2">
                <h4
                  className="px-3 pb-1 text-[10px] font-medium uppercase tracking-wider"
                  style={{ color: getCategoryColor(category) }}
                >
                  {getCategoryLabel(category)}
                </h4>
                <div className="flex flex-col gap-1">
                  {categoryEntries.map((entry) => (
                    <SearchResult
                      key={entry.id}
                      entry={entry}
                      query={submittedQuery}
                      maxScore={maxScore}
                    />
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function getCategoryLabel(category: string): string {
  switch (category) {
    case "Conscious":
      return "Pinned memories";
    case "Subconscious":
      return "Recalled";
    case "Novel":
      return "Connections";
    default:
      return category;
  }
}

function groupByCategory(
  entries: QueryIndexEntry[]
): Array<[string, QueryIndexEntry[]]> {
  const order = ["Conscious", "Subconscious", "Novel"];
  const groups = new Map<string, QueryIndexEntry[]>();

  // Sort by score descending first
  const sorted = [...entries].sort((a, b) => b.score - a.score);

  for (const entry of sorted) {
    const cat = entry.category;
    if (!groups.has(cat)) groups.set(cat, []);
    groups.get(cat)!.push(entry);
  }

  // Return known categories in defined order, then any unknown categories
  const result: Array<[string, QueryIndexEntry[]]> = [];
  for (const cat of order) {
    const group = groups.get(cat);
    if (group) result.push([cat, group]);
  }
  for (const [cat, group] of groups) {
    if (!order.includes(cat)) result.push([cat, group]);
  }
  return result;
}
