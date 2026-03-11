"use client";

import { MessagePrimitive, useAuiState } from "@assistant-ui/react";
import { StreamdownTextPrimitive } from "@assistant-ui/react-streamdown";
import { getContextForMessage, getQueryForMessage } from "@/lib/am-runtime";
import { MemoryPanel } from "./memory-panel";
import { StreamingError } from "./streaming-error";

export function UserMessage() {
  return (
    <MessagePrimitive.Root className="flex w-full max-w-2xl justify-end px-4 py-3">
      <div
        className="rounded-lg border px-4 py-3 max-w-[85%]"
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
      className="text-sm whitespace-pre-wrap"
      style={{ color: "var(--color-text-primary)" }}
    >
      <StreamdownTextPrimitive />
    </div>
  );
}

export function AssistantMessage() {
  const messageId = useAuiState((s) => s.message.id);
  const status = useAuiState((s) => s.message.status);
  const context = getContextForMessage(messageId);
  const userQuery = getQueryForMessage(messageId);

  // Narrow the discriminated union: only "incomplete" has reason and error
  const errorStatus =
    status?.type === "incomplete" && status.reason === "error"
      ? status
      : null;
  const borderColor = errorStatus
    ? "#ef444460"
    : "var(--color-assistant-border)";

  return (
    <MessagePrimitive.Root className="w-full max-w-2xl px-4 py-3">
      <div className="flex flex-col gap-1">
        <span
          className="text-xs font-medium"
          style={{ color: "var(--color-salient)" }}
        >
          AM
        </span>
        <div
          className="rounded-lg border px-4 py-3 max-w-[85%]"
          style={{
            borderColor,
            background: "var(--color-surface)",
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

/** Allow <salient> tags through the markdown sanitizer so AM recalled
 *  content renders with gold highlighting via the CSS rules in globals.css. */
const ALLOWED_TAGS = { salient: [] };

function AssistantTextPart() {
  return (
    <div
      className="prose prose-invert prose-sm max-w-none [&_pre]:rounded-md [&_pre]:border [&_code]:text-xs"
      style={{
        color: "var(--color-text-primary)",
        "--tw-prose-headings": "var(--color-text-primary)",
        "--tw-prose-links": "var(--color-salient)",
        "--tw-prose-code": "var(--color-text-primary)",
        "--tw-prose-pre-bg": "var(--color-surface-raised)",
        "--tw-prose-pre-code": "var(--color-text-primary)",
        "--tw-prose-borders": "var(--color-border)",
      } as React.CSSProperties}
    >
      <StreamdownTextPrimitive allowedTags={ALLOWED_TAGS} />
    </div>
  );
}
