"use client";

import { useState, useId } from "react";
import { Brain, ChevronDown, ChevronRight } from "lucide-react";
import type { ContextMetadata, RecallEntry } from "@/lib/types";
import { NeighborhoodCard } from "./neighborhood-card";

interface MemoryPanelProps {
  context: ContextMetadata | undefined;
  userQuery: string;
}

/**
 * Collapsible memory context panel shown below assistant messages.
 * Displays recalled topic clusters grouped by category.
 *
 * User-facing category labels:
 *   Conscious -> Pinned memory
 *   Subconscious -> Recalled
 *   Novel -> Connection
 */
export function MemoryPanel({ context, userQuery }: MemoryPanelProps) {
  const [expanded, setExpanded] = useState(false);
  const panelId = useId();

  if (!context?.index || context.index.length === 0) {
    return null;
  }

  const metrics = context.metrics;
  const entries = context.index;

  // Group by category
  const grouped = groupByCategory(entries);

  return (
    <div className="mt-1 max-w-[85%]">
      <button
        className="flex items-center gap-1.5 rounded px-2 py-1 text-[11px] transition-colors hover:opacity-80"
        style={{
          color: "var(--color-text-secondary)",
          background: expanded ? "var(--color-surface-raised)" : "transparent",
        }}
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
        aria-controls={panelId}
      >
        <Brain className="h-3 w-3" />
        <span>Memory</span>
        {metrics && (
          <span className="flex items-center gap-1.5">
            {metrics.conscious > 0 && (
              <CategoryCount
                label="pinned"
                count={metrics.conscious}
                color="var(--color-conscious)"
              />
            )}
            {metrics.subconscious > 0 && (
              <CategoryCount
                label="recalled"
                count={metrics.subconscious}
                color="var(--color-subconscious)"
              />
            )}
            {metrics.novel > 0 && (
              <CategoryCount
                label="connections"
                count={metrics.novel}
                color="var(--color-novel)"
              />
            )}
          </span>
        )}
        {expanded ? (
          <ChevronDown className="h-3 w-3" />
        ) : (
          <ChevronRight className="h-3 w-3" />
        )}
      </button>

      {expanded && (
        <div
          id={panelId}
          className="mt-2 flex flex-col gap-2"
          role="region"
          aria-label="Recalled memory context"
        >
          {grouped.conscious.length > 0 && (
            <CategorySection
              label="Pinned memory"
              color="var(--color-conscious)"
              entries={grouped.conscious}
              query={userQuery}
            />
          )}
          {grouped.subconscious.length > 0 && (
            <CategorySection
              label="Recalled"
              color="var(--color-subconscious)"
              entries={grouped.subconscious}
              query={userQuery}
            />
          )}
          {grouped.novel.length > 0 && (
            <CategorySection
              label="Connections"
              color="var(--color-novel)"
              entries={grouped.novel}
              query={userQuery}
            />
          )}
        </div>
      )}
    </div>
  );
}

function CategoryCount({
  label,
  count,
  color,
}: {
  label: string;
  count: number;
  color: string;
}) {
  return (
    <span className="flex items-center gap-0.5">
      <span
        className="inline-block h-1.5 w-1.5 rounded-full"
        style={{ background: color }}
      />
      <span>
        {count} {label}
      </span>
    </span>
  );
}

function CategorySection({
  label,
  color,
  entries,
  query,
}: {
  label: string;
  color: string;
  entries: RecallEntry[];
  query: string;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <span
        className="text-[10px] font-medium uppercase tracking-wider"
        style={{ color }}
      >
        {label}
      </span>
      {entries.map((entry) => (
        <NeighborhoodCard
          key={entry.id}
          entry={entry}
          query={query}
        />
      ))}
    </div>
  );
}

// --- Helpers ---

interface GroupedEntries {
  conscious: RecallEntry[];
  subconscious: RecallEntry[];
  novel: RecallEntry[];
}

function groupByCategory(entries: RecallEntry[]): GroupedEntries {
  const result: GroupedEntries = {
    conscious: [],
    subconscious: [],
    novel: [],
  };

  for (const entry of entries) {
    switch (entry.category) {
      case "Conscious":
        result.conscious.push(entry);
        break;
      case "Subconscious":
        result.subconscious.push(entry);
        break;
      case "Novel":
        result.novel.push(entry);
        break;
    }
  }

  return result;
}
