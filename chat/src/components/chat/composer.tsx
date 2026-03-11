"use client";

import { ComposerPrimitive } from "@assistant-ui/react";
import { SendHorizontal } from "lucide-react";

export function Composer() {
  return (
    <ComposerPrimitive.Root className="mx-auto w-full max-w-2xl px-4 pb-4 pt-2">
      <div
        className="flex items-end gap-2 rounded-xl border px-4 py-3 transition-colors focus-within:border-[var(--color-salient)]"
        style={{
          borderColor: "var(--color-border)",
          background: "var(--color-surface)",
          boxShadow: "var(--shadow-sm)",
        }}
      >
        <ComposerPrimitive.Input
          autoFocus
          placeholder="Message your memory..."
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
            className="flex h-8 w-8 items-center justify-center rounded-lg transition-all hover:scale-105 disabled:opacity-30 disabled:hover:scale-100"
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
