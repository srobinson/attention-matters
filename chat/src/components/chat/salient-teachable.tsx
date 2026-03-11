"use client";

import { useEffect, useState, useRef, useCallback } from "react";
import { X } from "lucide-react";

const STORAGE_KEY = "am_salient_tooltip_shown";

/**
 * One-time teachable moment for salient highlighted text.
 * Uses a MutationObserver to detect when the first <salient> element
 * appears in the chat thread, then shows a floating tooltip anchored
 * near it. Persists dismissal to localStorage so it only shows once.
 *
 * Mount this inside the chat thread viewport container.
 */
export function SalientTeachableMoment({
  containerRef,
}: {
  containerRef: React.RefObject<HTMLElement | null>;
}) {
  const [visible, setVisible] = useState(false);
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);
  const tooltipRef = useRef<HTMLDivElement>(null);
  const shownRef = useRef(false);

  const dismiss = useCallback(() => {
    setVisible(false);
    try {
      localStorage.setItem(STORAGE_KEY, "1");
    } catch {
      // localStorage unavailable
    }
  }, []);

  useEffect(() => {
    // Check if already shown
    try {
      if (localStorage.getItem(STORAGE_KEY) === "1") return;
    } catch {
      // localStorage unavailable, skip teachable moment
      return;
    }

    const container = containerRef.current;
    if (!container) return;

    const observer = new MutationObserver(() => {
      if (shownRef.current) return;

      const salientEl = container.querySelector("salient");
      if (!salientEl) return;

      shownRef.current = true;

      // Position the tooltip below the salient element
      const rect = salientEl.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      setPosition({
        top: rect.bottom - containerRect.top + 8,
        left: Math.max(8, rect.left - containerRect.left),
      });
      setVisible(true);

      observer.disconnect();
    });

    observer.observe(container, {
      childList: true,
      subtree: true,
    });

    return () => observer.disconnect();
  }, [containerRef]);

  if (!visible || !position) return null;

  return (
    <div
      ref={tooltipRef}
      className="absolute z-10 flex max-w-xs items-start gap-2 rounded-lg border px-3 py-2 shadow-lg"
      style={{
        top: position.top,
        left: position.left,
        borderColor: "var(--color-salient)",
        background: "var(--color-surface-raised)",
      }}
      role="tooltip"
    >
      <div className="flex-1">
        <p
          className="text-xs leading-relaxed"
          style={{ color: "var(--color-text-primary)" }}
        >
          <span
            className="font-medium"
            style={{ color: "var(--color-salient)" }}
          >
            Gold text
          </span>{" "}
          marks phrases AM considers important. These influence future memory recall.
        </p>
      </div>
      <button
        onClick={dismiss}
        className="mt-0.5 flex-shrink-0 rounded p-0.5 transition-colors hover:opacity-80"
        style={{ color: "var(--color-text-secondary)" }}
        aria-label="Dismiss tooltip"
      >
        <X className="h-3 w-3" />
      </button>
    </div>
  );
}
