"use client";

import { useState, useCallback, useEffect, useRef } from "react";
import { ThumbsUp, ThumbsDown, Loader2 } from "lucide-react";
import { amFeedback } from "@/lib/am-client";
import { loadSettings } from "@/lib/settings";

type FeedbackState =
  | "neutral"
  | "confirming-demote"
  | "sending"
  | "boosted"
  | "demoted"
  | "confirmed";

interface FeedbackButtonsProps {
  query: string;
  neighborhoodIds: string[];
  onFeedback?: () => void;
}

/**
 * Shared feedback buttons for rating recalled memories.
 * Used in: memory panel (per-message), episode detail, search results.
 *
 * User-facing terminology:
 *   boost -> "Helpful"
 *   demote -> "Not relevant"
 *
 * Demote requires confirmation step since the effect is harder to reverse.
 * Post-action shows brief inline "Memory updated" confirmation.
 */
export function FeedbackButtons({
  query,
  neighborhoodIds,
  onFeedback,
}: FeedbackButtonsProps) {
  const [state, setState] = useState<FeedbackState>("neutral");
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Clean up timer on unmount
  useEffect(() => {
    return () => {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
    };
  }, []);

  const disabled = neighborhoodIds.length === 0;

  const sendFeedback = useCallback(
    async (signal: "boost" | "demote") => {
      setState("sending");
      try {
        const apiKey = loadSettings().apiKey || undefined;
        await amFeedback(
          { query, neighborhood_ids: neighborhoodIds, signal },
          apiKey
        );
        setState(signal === "boost" ? "boosted" : "demoted");
        onFeedback?.();

        // Show "Memory updated" briefly, then settle into final state
        timerRef.current = setTimeout(() => setState("confirmed"), 2000);
      } catch {
        // Revert to neutral on failure so user can retry
        setState("neutral");
      }
    },
    [query, neighborhoodIds, onFeedback]
  );

  const handleBoost = useCallback(() => {
    sendFeedback("boost");
  }, [sendFeedback]);

  const handleDemoteIntent = useCallback(() => {
    setState("confirming-demote");
  }, []);

  const handleDemoteConfirm = useCallback(() => {
    sendFeedback("demote");
  }, [sendFeedback]);

  const handleDemoteCancel = useCallback(() => {
    setState("neutral");
  }, []);

  // Already rated and confirmed
  if (state === "confirmed") {
    return (
      <span
        className="text-[10px]"
        style={{ color: "var(--color-text-secondary)" }}
      >
        Rated
      </span>
    );
  }

  // Post-action confirmation with effect description
  if (state === "boosted" || state === "demoted") {
    return (
      <div className="flex flex-col gap-0.5">
        <span
          className="text-[10px] font-medium"
          style={{
            color:
              state === "boosted"
                ? "var(--color-novel)"
                : "var(--color-text-secondary)",
          }}
        >
          {state === "boosted"
            ? "Marked helpful"
            : "Marked not relevant"}
        </span>
        <span
          className="text-[10px]"
          style={{ color: "var(--color-text-secondary)", opacity: 0.7 }}
        >
          {state === "boosted"
            ? "AM will recall this topic cluster more often"
            : "AM will recall this topic cluster less often"}
        </span>
      </div>
    );
  }

  // Loading state
  if (state === "sending") {
    return (
      <Loader2
        className="h-3 w-3 animate-spin"
        style={{ color: "var(--color-text-secondary)" }}
      />
    );
  }

  // Demote confirmation step
  if (state === "confirming-demote") {
    return (
      <div className="flex items-center gap-1">
        <span
          className="text-[10px]"
          style={{ color: "var(--color-text-secondary)" }}
        >
          Are you sure?
        </span>
        <button
          onClick={handleDemoteConfirm}
          className="rounded px-1.5 py-0.5 text-[10px] font-medium transition-colors hover:opacity-80"
          style={{
            background: "var(--color-surface-raised)",
            color: "var(--color-text-primary)",
          }}
        >
          Yes
        </button>
        <button
          onClick={handleDemoteCancel}
          className="rounded px-1.5 py-0.5 text-[10px] transition-colors hover:opacity-80"
          style={{
            color: "var(--color-text-secondary)",
          }}
        >
          Cancel
        </button>
      </div>
    );
  }

  // Default neutral state with scope label
  return (
    <div className="flex flex-col gap-1">
      <span
        className="text-[10px]"
        style={{ color: "var(--color-text-secondary)" }}
      >
        Rate this topic cluster
      </span>
      <div className="flex items-center gap-1">
        <button
          onClick={handleBoost}
          disabled={disabled}
          className="flex items-center gap-1 rounded px-2 py-1 text-[10px] transition-colors hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-40"
          style={{
            color: "var(--color-text-secondary)",
            background: "var(--color-surface-raised)",
          }}
          title="Mark this topic cluster as helpful. AM will surface it more often in future sessions."
          aria-label="Mark this topic cluster as helpful"
        >
          <ThumbsUp className="h-3 w-3" />
          <span>Helpful</span>
        </button>
        <button
          onClick={handleDemoteIntent}
          disabled={disabled}
          className="flex items-center gap-1 rounded px-2 py-1 text-[10px] transition-colors hover:opacity-80 disabled:cursor-not-allowed disabled:opacity-40"
          style={{
            color: "var(--color-text-secondary)",
            background: "var(--color-surface-raised)",
          }}
          title="Mark this topic cluster as not relevant. AM will surface it less often in future sessions."
          aria-label="Mark this topic cluster as not relevant"
        >
          <ThumbsDown className="h-3 w-3" />
          <span>Not relevant</span>
        </button>
      </div>
    </div>
  );
}
