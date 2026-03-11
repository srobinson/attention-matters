"use client";

import { useState, useId } from "react";
import { Brain, ChevronDown, ChevronRight, Pin, History, Sparkles, type LucideIcon } from "lucide-react";
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
    <div className="mt-1.5 max-w-[85%]">
      <button
        className="flex items-center gap-1.5 rounded-md px-2 py-1.5 transition-all hover:bg-[var(--color-surface-raised)]"
        style={{
          color: "var(--color-text-secondary)",
          background: expanded ? "var(--color-surface-raised)" : "transparent",
          fontSize: "var(--font-size-xs)",
        }}
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
        aria-controls={panelId}
      >
        <Brain className="h-3 w-3" />
        <span>Memory context</span>
        {metrics && (
          <span className="flex items-center gap-1.5">
            {metrics.conscious > 0 && (
              <CategoryCount
                label="pinned"
                count={metrics.conscious}
                color="var(--color-conscious)"
                icon={Pin}
              />
            )}
            {metrics.subconscious > 0 && (
              <CategoryCount
                label="recalled"
                count={metrics.subconscious}
                color="var(--color-subconscious)"
                icon={History}
              />
            )}
            {metrics.novel > 0 && (
              <CategoryCount
                label="connections"
                count={metrics.novel}
                color="var(--color-novel)"
                icon={Sparkles}
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
          className="animate-expand mt-2 flex flex-col gap-2 rounded-sm border-l-2 pl-3"
          style={{ borderColor: "var(--color-salient)" }}
          role="region"
          aria-label="Recalled memory context for this response"
        >
          <span
            style={{
              color: "var(--color-text-tertiary)",
              fontSize: "var(--font-size-micro)",
            }}
          >
            What AM recalled for this response
          </span>
          {grouped.conscious.length > 0 && (
            <CategorySection
              label="Pinned memory"
              color="var(--color-conscious)"
              icon={Pin}
              entries={grouped.conscious}
              query={userQuery}
            />
          )}
          {grouped.subconscious.length > 0 && (
            <CategorySection
              label="Recalled"
              color="var(--color-subconscious)"
              icon={History}
              entries={grouped.subconscious}
              query={userQuery}
            />
          )}
          {grouped.novel.length > 0 && (
            <CategorySection
              label="Connections"
              color="var(--color-novel)"
              icon={Sparkles}
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
  icon: Icon,
}: {
  label: string;
  count: number;
  color: string;
  icon: LucideIcon;
}) {
  return (
    <span className="flex items-center gap-0.5">
      <Icon className="h-2.5 w-2.5" style={{ color }} aria-hidden="true" />
      <span>
        {count} {label}
      </span>
    </span>
  );
}

function CategorySection({
  label,
  color,
  icon: Icon,
  entries,
  query,
}: {
  label: string;
  color: string;
  icon: LucideIcon;
  entries: RecallEntry[];
  query: string;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <span
        className="flex items-center gap-1 font-semibold uppercase"
        style={{
          color,
          fontSize: "var(--font-size-micro)",
          letterSpacing: "var(--tracking-wider)",
        }}
      >
        <Icon className="h-2.5 w-2.5" aria-hidden="true" />
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
