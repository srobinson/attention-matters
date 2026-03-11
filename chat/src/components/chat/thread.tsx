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
    <div className="animate-fade-in flex items-center gap-3 py-2">
      <div
        className="h-px flex-1"
        style={{ background: "var(--color-border)" }}
      />
      <span
        className="font-medium uppercase"
        style={{
          color: "var(--color-text-tertiary)",
          fontSize: "var(--font-size-micro)",
          letterSpacing: "var(--tracking-wider)",
        }}
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
    <div className="flex flex-1 flex-col items-center justify-center gap-5 px-4">
      {/* Subtle gold accent mark */}
      <div
        className="flex h-12 w-12 items-center justify-center rounded-xl"
        style={{
          background: "var(--color-salient-glow)",
          boxShadow: "var(--shadow-glow-gold)",
        }}
      >
        <span
          className="font-semibold"
          style={{
            color: "var(--color-salient)",
            fontSize: "var(--font-size-lg)",
            letterSpacing: "var(--tracking-tight)",
          }}
        >
          AM
        </span>
      </div>
      <div className="flex flex-col items-center gap-2">
        <h1
          className="font-semibold"
          style={{
            color: "var(--color-text-primary)",
            fontSize: "var(--font-size-2xl)",
            letterSpacing: "var(--tracking-tight)",
            lineHeight: "var(--line-height-tight)",
          }}
        >
          Converse with your memory
        </h1>
        <p
          className="max-w-md text-center"
          style={{
            color: "var(--color-text-secondary)",
            fontSize: "var(--font-size-sm)",
            lineHeight: "var(--line-height-relaxed)",
          }}
        >
          Ask questions, explore recalled context, and build understanding
          through dialogue.
        </p>
      </div>
    </div>
  );
}
