"use client";

import { useMemo, useCallback, useState, useEffect } from "react";
import {
  AssistantRuntimeProvider,
  useLocalRuntime,
} from "@assistant-ui/react";
import { ChatThread } from "@/components/chat/thread";
import { createAMAdapter } from "@/lib/am-runtime";
import { mockAdapter } from "@/lib/runtime";
import { loadSettings, hasApiKey } from "@/lib/settings";

export default function ChatPage() {
  const [connected, setConnected] = useState(false);

  // Check for API key on mount
  useEffect(() => {
    setConnected(hasApiKey());
  }, []);

  const getApiKey = useCallback(() => loadSettings().apiKey || undefined, []);
  const getModel = useCallback(() => loadSettings().model || undefined, []);
  const getMode = useCallback(() => loadSettings().mode, []);

  const adapter = useMemo(() => {
    if (!connected) return mockAdapter;
    return createAMAdapter({
      getApiKey,
      getModel,
      getMode,
    });
  }, [connected, getApiKey, getModel, getMode]);

  const runtime = useLocalRuntime(adapter);

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
        <Header connected={connected} />
        <ChatThread />
      </main>
    </AssistantRuntimeProvider>
  );
}

function Header({ connected }: { connected: boolean }) {
  return (
    <header
      className="flex items-center justify-between border-b px-4"
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
      <div className="flex items-center gap-2">
        <span
          className="text-xs"
          style={{
            color: connected
              ? "var(--color-novel)"
              : "var(--color-text-secondary)",
          }}
        >
          {connected ? "Connected" : "Mock mode"}
        </span>
        <div
          className="h-2 w-2 rounded-full"
          style={{
            background: connected
              ? "var(--color-novel)"
              : "var(--color-text-secondary)",
          }}
        />
      </div>
    </header>
  );
}
