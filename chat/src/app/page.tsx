"use client";

import {
  AssistantRuntimeProvider,
  useLocalRuntime,
} from "@assistant-ui/react";
import { ChatThread } from "@/components/chat/chat-thread";
import { mockAdapter } from "@/lib/runtime";

export default function ChatPage() {
  const runtime = useLocalRuntime(mockAdapter);

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <main
        className="grid h-dvh"
        style={{
          gridTemplateRows: "var(--header-height) 1fr",
          gridTemplateColumns: "1fr",
          background: "var(--color-bg)",
        }}
      >
        <Header />
        <ChatThread />
      </main>
    </AssistantRuntimeProvider>
  );
}

function Header() {
  return (
    <header
      className="flex items-center border-b px-4"
      style={{
        height: "var(--header-height)",
        borderColor: "var(--color-border)",
        background: "var(--color-surface)",
      }}
    >
      <h1
        className="text-sm font-medium"
        style={{ color: "var(--color-text-primary)" }}
      >
        AM Chat
      </h1>
    </header>
  );
}
