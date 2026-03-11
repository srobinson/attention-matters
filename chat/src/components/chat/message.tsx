"use client";

import { MessagePrimitive } from "@assistant-ui/react";
import { StreamdownTextPrimitive } from "@assistant-ui/react-streamdown";

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
            borderColor: "var(--color-assistant-border)",
            background: "var(--color-surface)",
          }}
        >
          <MessagePrimitive.Content
            components={{
              Text: AssistantTextPart,
            }}
          />
        </div>
      </div>
    </MessagePrimitive.Root>
  );
}

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
      <StreamdownTextPrimitive />
    </div>
  );
}
