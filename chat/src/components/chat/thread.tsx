"use client";

import { useRef } from "react";
import { ThreadPrimitive } from "@assistant-ui/react";
import { UserMessage, AssistantMessage } from "./message";
import { Composer } from "./composer";
import { SalientTeachableMoment } from "./salient-teachable";

interface ChatThreadProps {
  modeNotices?: string[];
}

export function ChatThread({ modeNotices }: ChatThreadProps) {
  const viewportRef = useRef<HTMLDivElement>(null);

  return (
    <ThreadPrimitive.Root
      className="flex h-full flex-col"
      style={{ background: "var(--color-bg)" }}
    >
      <ThreadPrimitive.Viewport
        ref={viewportRef}
        className="relative flex flex-1 flex-col items-center overflow-y-auto scroll-smooth"
      >
        <ThreadPrimitive.Empty>
          <EmptyState />
        </ThreadPrimitive.Empty>

        <ThreadPrimitive.Messages
          components={{
            UserMessage,
            AssistantMessage,
          }}
        />

        {/* Mode switch system notices */}
        {modeNotices && modeNotices.length > 0 && (
          <div className="flex w-full max-w-2xl flex-col gap-1 px-4 py-2">
            {modeNotices.map((notice, i) => (
              <ModeNotice key={i} text={notice} />
            ))}
          </div>
        )}

        <SalientTeachableMoment containerRef={viewportRef} />
      </ThreadPrimitive.Viewport>

      <Composer />
    </ThreadPrimitive.Root>
  );
}

function ModeNotice({ text }: { text: string }) {
  return (
    <div className="flex items-center gap-3 py-1">
      <div
        className="h-px flex-1"
        style={{ background: "var(--color-border)" }}
      />
      <span
        className="text-[10px] font-medium uppercase tracking-wider"
        style={{ color: "var(--color-text-secondary)" }}
      >
        {text}
      </span>
      <div
        className="h-px flex-1"
        style={{ background: "var(--color-border)" }}
      />
    </div>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-4 px-4">
      <div
        className="text-2xl font-semibold"
        style={{ color: "var(--color-text-primary)" }}
      >
        AM Chat
      </div>
      <p
        className="max-w-md text-center text-sm"
        style={{ color: "var(--color-text-secondary)" }}
      >
        Converse with your memory. Ask questions, explore recalled context, and
        build understanding through dialogue.
      </p>
    </div>
  );
}
