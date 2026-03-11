"use client";

import { ComposerPrimitive } from "@assistant-ui/react";
import { SendHorizontal } from "lucide-react";

export function Composer() {
  return (
    <ComposerPrimitive.Root className="mx-auto w-full max-w-2xl px-4 pb-4">
      <div
        className="flex items-end gap-2 rounded-lg border px-3 py-2"
        style={{
          borderColor: "var(--color-border)",
          background: "var(--color-surface)",
        }}
      >
        <ComposerPrimitive.Input
          autoFocus
          placeholder="Message your memory..."
          rows={1}
          className="flex-1 resize-none bg-transparent text-sm outline-none placeholder:opacity-50 min-h-6 max-h-[200px]"
          style={{
            color: "var(--color-text-primary)",
          }}
        />
        <ComposerPrimitive.Send asChild>
          <button
            className="flex h-8 w-8 items-center justify-center rounded-md transition-colors hover:opacity-80 disabled:opacity-30"
            style={{ color: "var(--color-salient)" }}
            aria-label="Send message"
          >
            <SendHorizontal className="h-4 w-4" />
          </button>
        </ComposerPrimitive.Send>
      </div>
    </ComposerPrimitive.Root>
  );
}
