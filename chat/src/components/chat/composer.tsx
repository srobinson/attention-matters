"use client";

import { ComposerPrimitive } from "@assistant-ui/react";
import { SendHorizontal } from "lucide-react";

export function Composer() {
  return (
    <ComposerPrimitive.Root className="mx-auto w-full max-w-(--content-wide-max-width) px-3 pb-4 pt-2 sm:px-4">
      <div
        className="mb-2 flex items-center justify-between gap-3 px-1"
        style={{
          color: "var(--color-text-tertiary)",
          fontSize: "var(--font-size-xs)",
        }}
      >
        <span>Ask for patterns, summaries, or specific recalled moments.</span>
        <span className="hidden sm:inline">Shift+Enter for a new line</span>
      </div>
      <div
        className="flex items-end gap-3 rounded-[1.4rem] border px-4 py-3 transition-colors focus-within:border-[var(--color-salient)]"
        style={{
          borderColor: "var(--color-border)",
          background: "color-mix(in srgb, var(--color-surface) 90%, transparent)",
          boxShadow: "var(--shadow-sm)",
          backdropFilter: "blur(12px)",
        }}
      >
        <ComposerPrimitive.Input
          autoFocus
          placeholder="Ask what your memory is holding onto..."
          rows={1}
          className="min-h-6 max-h-[200px] flex-1 resize-none bg-transparent outline-none placeholder:opacity-40"
          style={{
            color: "var(--color-text-primary)",
            fontSize: "var(--font-size-base)",
            lineHeight: "var(--line-height-normal)",
          }}
        />
        <ComposerPrimitive.Send asChild>
          <button
            className="flex h-10 w-10 items-center justify-center rounded-xl transition-all hover:scale-105 disabled:opacity-30 disabled:hover:scale-100"
            style={{
              color: "var(--color-salient)",
              background: "var(--color-salient-glow)",
            }}
            aria-label="Send message"
          >
            <SendHorizontal className="h-4 w-4" />
          </button>
        </ComposerPrimitive.Send>
      </div>
    </ComposerPrimitive.Root>
  );
}
