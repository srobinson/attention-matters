"use client";

import { useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { FeedbackButtons } from "./feedback-buttons";
import { getCategoryColor } from "./shared";

type SortField = "type" | "tokens";

interface NeighborhoodEntry {
  id: string;
  category: string;
  type: string;
  episode: string;
  tokens: number;
  text: string;
}

interface NeighborhoodListProps {
  neighborhoods: NeighborhoodEntry[];
  query: string;
}

/**
 * Renders a list of topic clusters with collapsible source text,
 * type badge, token count, and feedback controls.
 * User-facing terminology: "topic cluster" (not "neighborhood").
 */
export function NeighborhoodList({ neighborhoods, query }: NeighborhoodListProps) {
  const [sortBy, setSortBy] = useState<SortField>("type");

  const sorted = [...neighborhoods].sort((a, b) => {
    if (sortBy === "tokens") return b.tokens - a.tokens;
    return a.type.localeCompare(b.type);
  });

  if (neighborhoods.length === 0) {
    return (
      <p
        className="px-3 py-4 text-center text-xs"
        style={{ color: "var(--color-text-secondary)" }}
      >
        No topic clusters found.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-1">
      {/* Sort controls */}
      <div className="flex items-center gap-2 px-3 pb-1">
        <span
          className="text-[10px]"
          style={{ color: "var(--color-text-secondary)" }}
        >
          Sort:
        </span>
        <SortButton
          active={sortBy === "type"}
          label="Type"
          onClick={() => setSortBy("type")}
        />
        <SortButton
          active={sortBy === "tokens"}
          label="Size"
          onClick={() => setSortBy("tokens")}
        />
      </div>

      {sorted.map((entry) => (
        <NeighborhoodItem key={entry.id} entry={entry} query={query} />
      ))}
    </div>
  );
}

function NeighborhoodItem({
  entry,
  query,
}: {
  entry: NeighborhoodEntry;
  query: string;
}) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div
      className="mx-2 rounded-md border px-3 py-2"
      style={{
        borderColor: "var(--color-border-subtle)",
        background: "var(--color-surface)",
      }}
    >
      <button
        className="flex w-full items-start justify-between gap-2 text-left"
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
        aria-controls={`nb-${entry.id}`}
      >
        <div className="flex flex-1 flex-col gap-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            {/* Type badge */}
            <span
              className="inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider"
              style={{
                background: `${getCategoryColor(entry.category)}20`,
                color: getCategoryColor(entry.category),
              }}
            >
              {entry.type}
            </span>

            {/* Token count */}
            <span
              className="text-[10px] tabular-nums"
              style={{ color: "var(--color-text-secondary)" }}
            >
              {entry.tokens} tokens
            </span>

            {/* Episode source */}
            <span
              className="truncate text-[10px]"
              style={{ color: "var(--color-text-secondary)" }}
            >
              {entry.episode}
            </span>
          </div>

          {/* Preview text (truncated when collapsed) */}
          <p
            className="text-xs leading-relaxed"
            style={{ color: "var(--color-text-primary)" }}
          >
            {expanded ? "" : truncate(entry.text, 100)}
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

      {/* Expanded content */}
      {expanded && (
        <div
          id={`nb-${entry.id}`}
          className="mt-2 border-t pt-2"
          style={{ borderColor: "var(--color-border-subtle)" }}
        >
          <pre
            className="mb-2 whitespace-pre-wrap text-xs leading-relaxed"
            style={{ color: "var(--color-text-primary)" }}
          >
            {entry.text}
          </pre>

          <FeedbackButtons
            query={query}
            neighborhoodIds={[entry.id]}
          />
        </div>
      )}
    </div>
  );
}

function SortButton({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="rounded px-1.5 py-0.5 text-[10px] font-medium transition-colors"
      style={{
        background: active ? "var(--color-surface-raised)" : "transparent",
        color: active
          ? "var(--color-text-primary)"
          : "var(--color-text-secondary)",
      }}
    >
      {label}
    </button>
  );
}

function truncate(text: string, maxLength: number): string {
  if (text.length <= maxLength) return text;
  return text.slice(0, maxLength).trimEnd() + "...";
}

export type { NeighborhoodEntry };
