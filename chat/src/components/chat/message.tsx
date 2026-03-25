"use client";

import { MessagePrimitive, useAuiState } from "@assistant-ui/react";
import { StreamdownTextPrimitive } from "@assistant-ui/react-streamdown";
import { getContextForMessage, getQueryForMessage } from "@/lib/am-runtime";
import { MemoryPanel } from "./memory-panel";
import { StreamingError } from "./streaming-error";

export function UserMessage() {
  return (
    <MessagePrimitive.Root className="animate-message-enter flex w-full max-w-2xl justify-end px-4 py-2">
      <div
        className="max-w-[85%] rounded-lg border px-4 py-3"
        style={{
          borderColor: "var(--color-user-border)",
          background: "var(--color-surface)",
        }}
      >
        <MessagePrimitive.Content
          components={{
            Text: UserTextPart,
          }}
        />
      </div>
    </MessagePrimitive.Root>
  );
}

function UserTextPart() {
  return (
    <div
      className="whitespace-pre-wrap"
      style={{
        color: "var(--color-text-primary)",
        fontSize: "var(--font-size-base)",
        lineHeight: "var(--line-height-relaxed)",
      }}
    >
      <StreamdownTextPrimitive />
    </div>
  );
}

export function AssistantMessage() {
  const messageId = useAuiState((s) => s.message.id);
  const status = useAuiState((s) => s.message.status);
  const content = useAuiState((s) => s.message.content);
  const context = getContextForMessage(messageId);
  const userQuery = getQueryForMessage(messageId);

  // Narrow the discriminated union: only "incomplete" has reason and error
  const errorStatus =
    status?.type === "incomplete" && status.reason === "error"
      ? status
      : null;

  // Thinking state: message is running but has no text content yet
  const isRunning = status?.type === "running";
  const hasContent = content.some(
    (part) => part.type === "text" && part.text.length > 0
  );
  const isThinking = isRunning && !hasContent;

  // Border class: thinking animation or default
  const borderClass = isThinking ? "thinking-border" : "";

  const borderStyle = isThinking
    ? {} // thinking-border class handles border via CSS
    : { borderColor: errorStatus ? "var(--color-error-muted)" : "var(--color-assistant-border)" };

  return (
    <MessagePrimitive.Root className="animate-message-enter w-full max-w-2xl px-4 py-2">
      <div className="flex flex-col gap-1.5">
        <span
          className="font-semibold uppercase"
          style={{
            color: "var(--color-salient)",
            fontSize: "var(--font-size-micro)",
            letterSpacing: "var(--tracking-wider)",
          }}
        >
          AM
        </span>
        <div
          className={`max-w-[85%] rounded-lg border px-4 py-3 transition-[border-color,box-shadow,background] duration-200 ${borderClass} ${
            isThinking ? "min-h-14" : ""
          }`}
          data-thinking={isThinking ? "true" : "false"}
          aria-busy={isThinking}
          style={{
            ...borderStyle,
            backgroundColor: "var(--color-surface)",
          }}
        >
          <MessagePrimitive.Content
            components={{
              Text: AssistantTextPart,
            }}
          />
          {errorStatus && <StreamingError error={errorStatus.error} />}
        </div>
        <MemoryPanel context={context} userQuery={userQuery} />
      </div>
    </MessagePrimitive.Root>
  );
}

/** Allow <salient> tags through the sanitizer and render them as
 *  <span class="salient-tag"> to avoid React DOM warnings while
 *  preserving the gold highlight styling from globals.css. */
const SALIENT_ALLOWED: Record<string, string[]> = { salient: [] };

function SalientTag({ children }: { children?: React.ReactNode }) {
  return <span className="salient-tag">{children}</span>;
}

function AssistantTextPart() {
  return (
    <div
      className="prose prose-invert prose-sm max-w-none [&_pre]:rounded-md [&_pre]:border [&_code]:text-xs [&_p]:leading-relaxed"
      style={{
        color: "var(--color-text-primary)",
        fontSize: "var(--font-size-base)",
        "--tw-prose-headings": "var(--color-text-primary)",
        "--tw-prose-links": "var(--color-salient)",
        "--tw-prose-code": "var(--color-text-primary)",
        "--tw-prose-pre-bg": "var(--color-surface-raised)",
        "--tw-prose-pre-code": "var(--color-text-primary)",
        "--tw-prose-borders": "var(--color-border)",
      } as React.CSSProperties}
    >
      <StreamdownTextPrimitive
        allowedTags={SALIENT_ALLOWED}
        components={{ salient: SalientTag } as Record<string, React.ComponentType>}
      />
    </div>
  );
}
