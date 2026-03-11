"use client";

import { useState } from "react";
import { ChevronDown, ChevronRight, ThumbsUp, ThumbsDown } from "lucide-react";
import type { RecallEntry } from "@/lib/types";

interface NeighborhoodCardProps {
  entry: RecallEntry;
  onFeedback?: (id: string, signal: "boost" | "demote") => void;
}

/**
 * Displays a single recalled topic cluster with category badge,
 * match strength, generation, and expandable source text.
 *
 * Uses user-facing terminology per ALP-1127 mapping:
 *   neighborhood -> topic cluster
 *   epoch -> generation
 *   activation count -> times recalled
 *   boost -> helpful
 *   demote -> not relevant
 */
export function NeighborhoodCard({ entry, onFeedback }: NeighborhoodCardProps) {
  const [expanded, setExpanded] = useState(false);
  const strength = scoreToStrength(entry.score);
  const categoryColor = getCategoryColor(entry.category);

  return (
    <div
      className="rounded-md border px-3 py-2"
      style={{
        borderColor: "var(--color-border-subtle)",
        background: "var(--color-surface)",
      }}
    >
      <button
        className="flex w-full items-start justify-between gap-2 text-left"
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
        aria-controls={`recall-${entry.id}`}
      >
        <div className="flex flex-1 flex-col gap-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            {/* Type badge */}
            <span
              className="inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider"
              style={{
                background: `${categoryColor}20`,
                color: categoryColor,
              }}
            >
              {entry.type}
            </span>

            {/* Match strength */}
            <span
              className="text-[10px] font-medium"
              style={{ color: strengthColor(strength) }}
            >
              {strength}
            </span>

            {/* Generation */}
            <span
              className="text-[10px]"
              style={{ color: "var(--color-text-secondary)" }}
            >
              Gen {entry.epoch}
            </span>
          </div>

          {/* Summary (truncated when collapsed) */}
          <p
            className="text-xs leading-relaxed"
            style={{ color: "var(--color-text-primary)" }}
          >
            {expanded
              ? entry.summary
              : truncate(entry.summary, 120)}
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
          id={`recall-${entry.id}`}
          className="mt-2 flex items-center gap-1 border-t pt-2"
          style={{ borderColor: "var(--color-border-subtle)" }}
        >
          {onFeedback && (
            <>
              <FeedbackButton
                icon={<ThumbsUp className="h-3 w-3" />}
                label="Helpful"
                onClick={() => onFeedback(entry.id, "boost")}
              />
              <FeedbackButton
                icon={<ThumbsDown className="h-3 w-3" />}
                label="Not relevant"
                onClick={() => onFeedback(entry.id, "demote")}
              />
            </>
          )}
        </div>
      )}
    </div>
  );
}

function FeedbackButton({
  icon,
  label,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      className="flex items-center gap-1 rounded px-2 py-1 text-[10px] transition-colors hover:opacity-80"
      style={{
        color: "var(--color-text-secondary)",
        background: "var(--color-surface-raised)",
      }}
      onClick={onClick}
      title={label}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}

// --- Helpers ---

function scoreToStrength(score: number): string {
  if (score >= 7) return "Strong match";
  if (score >= 4) return "Moderate match";
  return "Weak match";
}

function strengthColor(strength: string): string {
  if (strength.startsWith("Strong")) return "var(--color-conscious)";
  if (strength.startsWith("Moderate")) return "var(--color-subconscious)";
  return "var(--color-text-secondary)";
}

function getCategoryColor(category: string): string {
  switch (category) {
    case "Conscious":
      return "var(--color-conscious)";
    case "Subconscious":
      return "var(--color-subconscious)";
    case "Novel":
      return "var(--color-novel)";
    default:
      return "var(--color-text-secondary)";
  }
}

function truncate(text: string, maxLength: number): string {
  if (text.length <= maxLength) return text;
  return text.slice(0, maxLength).trimEnd() + "...";
}
