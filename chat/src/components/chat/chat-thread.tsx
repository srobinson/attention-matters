"use client";

import {
  ThreadPrimitive,
  ComposerPrimitive,
  MessagePrimitive,
} from "@assistant-ui/react";
import { StreamdownTextPrimitive } from "@assistant-ui/react-streamdown";
import { SendHorizontal } from "lucide-react";

export function ChatThread() {
  return (
    <ThreadPrimitive.Root
      className="flex h-full flex-col"
      style={{ background: "var(--color-bg)" }}
    >
      <ThreadPrimitive.Viewport className="flex flex-1 flex-col items-center overflow-y-auto scroll-smooth">
        <ThreadPrimitive.Empty>
          <EmptyState />
        </ThreadPrimitive.Empty>

        <ThreadPrimitive.Messages
          components={{
            UserMessage,
            AssistantMessage,
          }}
        />
      </ThreadPrimitive.Viewport>

      <Composer />
    </ThreadPrimitive.Root>
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

function UserMessage() {
  return (
    <MessagePrimitive.Root className="w-full max-w-2xl px-4 py-3">
      <div
        className="rounded-lg border px-4 py-3 ml-auto max-w-[85%]"
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

function AssistantMessage() {
  return (
    <MessagePrimitive.Root className="w-full max-w-2xl px-4 py-3">
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
    </MessagePrimitive.Root>
  );
}

function AssistantTextPart() {
  return (
    <div
      className="prose prose-invert prose-sm max-w-none"
      style={{ color: "var(--color-text-primary)" }}
    >
      <StreamdownTextPrimitive />
    </div>
  );
}

function Composer() {
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
          >
            <SendHorizontal className="h-4 w-4" />
          </button>
        </ComposerPrimitive.Send>
      </div>
    </ComposerPrimitive.Root>
  );
}
