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
      style={{ background: "transparent" }}
    >
      <ThreadPrimitive.Viewport
        ref={viewportRef}
        className="relative flex flex-1 flex-col items-center overflow-y-auto scroll-smooth px-2 pb-4 sm:px-4"
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
    <div className="flex flex-1 flex-col justify-center gap-8 px-4 py-10">
      <div className="mx-auto flex w-full max-w-(--content-wide-max-width) flex-col gap-8">
        <div className="flex flex-col gap-4">
          <div
            className="inline-flex w-fit items-center gap-2 rounded-full border px-3 py-1"
            style={{
              borderColor: "var(--color-border)",
              background: "color-mix(in srgb, var(--color-surface-raised) 80%, transparent)",
              color: "var(--color-salient)",
              fontSize: "var(--font-size-xs)",
            }}
          >
            <span
              className="h-1.5 w-1.5 rounded-full"
              style={{ background: "var(--color-salient)" }}
            />
            Context-rich dialogue
          </div>
          <div className="grid gap-6 lg:grid-cols-[minmax(0,1.25fr)_minmax(280px,0.75fr)] lg:items-end">
            <div className="flex flex-col gap-4">
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
              <div className="flex flex-col gap-3">
                <h1
                  className="max-w-2xl font-semibold"
                  style={{
                    color: "var(--color-text-primary)",
                    fontSize: "clamp(2rem, 3vw, 3rem)",
                    letterSpacing: "var(--tracking-tight)",
                    lineHeight: 0.96,
                  }}
                >
                  Ask a question. AM will answer with recalled context, patterns,
                  and memory traces in view.
                </h1>
                <p
                  className="max-w-xl"
                  style={{
                    color: "var(--color-text-secondary)",
                    fontSize: "var(--font-size-base)",
                    lineHeight: "var(--line-height-relaxed)",
                  }}
                >
                  This workspace is built for exploratory dialogue: pinned
                  memories, latent recalls, and novel connections are surfaced
                  alongside the response so you can inspect how the answer was
                  formed.
                </p>
              </div>
            </div>

            <div
              className="rounded-[1.5rem] border p-4 sm:p-5"
              style={{
                borderColor: "var(--color-border)",
                background: "color-mix(in srgb, var(--color-surface) 88%, transparent)",
              }}
            >
              <div className="mb-4 flex items-center justify-between gap-3">
                <div>
                  <div
                    className="font-semibold uppercase"
                    style={{
                      color: "var(--color-text-tertiary)",
                      fontSize: "var(--font-size-micro)",
                      letterSpacing: "var(--tracking-wider)",
                    }}
                  >
                    Good first prompts
                  </div>
                  <p
                    style={{
                      color: "var(--color-text-secondary)",
                      fontSize: "var(--font-size-xs)",
                      lineHeight: "var(--line-height-relaxed)",
                    }}
                  >
                    Prime the system with questions that benefit from retrieval.
                  </p>
                </div>
              </div>
              <div className="flex flex-col gap-2.5">
                {[
                  "What patterns keep showing up in my recent conversations?",
                  "Summarize the key memories related to project planning.",
                  "What did I pin as especially important lately?",
                ].map((prompt) => (
                  <div
                    key={prompt}
                    className="rounded-xl border px-3 py-3"
                    style={{
                      borderColor: "var(--color-border)",
                      background: "var(--color-surface-raised)",
                      color: "var(--color-text-secondary)",
                      fontSize: "var(--font-size-sm)",
                      lineHeight: "var(--line-height-relaxed)",
                    }}
                  >
                    {prompt}
                  </div>
                ))}
              </div>
            </div>
          </div>
        </div>

        <div className="grid gap-3 sm:grid-cols-3">
          <FeatureTile
            title="Pinned memory"
            body="High-salience memories stay visible and traceable inside each response."
          />
          <FeatureTile
            title="Recalled context"
            body="AM pulls latent context from prior episodes instead of relying on manual context stuffing."
          />
          <FeatureTile
            title="Novel connections"
            body="Unexpected related clusters surface as connections, not noise."
          />
        </div>
      </div>
    </div>
  );
}

function FeatureTile({
  title,
  body,
}: {
  title: string;
  body: string;
}) {
  return (
    <div
      className="rounded-[1.25rem] border p-4"
      style={{
        borderColor: "var(--color-border)",
        background: "color-mix(in srgb, var(--color-surface-raised) 80%, transparent)",
      }}
    >
      <div className="mb-2">
        <span
          className="font-semibold uppercase"
          style={{
            color: "var(--color-salient)",
            fontSize: "var(--font-size-micro)",
            letterSpacing: "var(--tracking-wider)",
          }}
        >
          {title}
        </span>
      </div>
      <p
        style={{
          color: "var(--color-text-secondary)",
          fontSize: "var(--font-size-sm)",
          lineHeight: "var(--line-height-relaxed)",
        }}
      >
        {body}
      </p>
    </div>
  );
}
