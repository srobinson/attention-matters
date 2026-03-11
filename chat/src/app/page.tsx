"use client";

import { useMemo, useCallback, useState, useEffect, useRef } from "react";
import {
  AssistantRuntimeProvider,
  useLocalRuntime,
} from "@assistant-ui/react";
import { ChatThread } from "@/components/chat/thread";
import { SetupCard } from "@/components/settings/setup-card";
import { SettingsBar } from "@/components/settings/settings-bar";
import { Sidebar } from "@/components/memory/sidebar";
import { UploadModal } from "@/components/upload/upload-modal";
import { createAMAdapter } from "@/lib/am-runtime";
import { mockAdapter } from "@/lib/runtime";
import { loadSettings, hasApiKey } from "@/lib/settings";
import { useQueryClient } from "@tanstack/react-query";

export default function ChatPage() {
  const [connected, setConnected] = useState(false);
  const [settingsVersion, setSettingsVersion] = useState(0);
  const [showUpload, setShowUpload] = useState(false);
  const [modeNotices, setModeNotices] = useState<string[]>([]);
  const prevModeRef = useRef<string | null>(null);
  const queryClient = useQueryClient();

  // Check for API key on mount, initialize mode tracking
  useEffect(() => {
    setConnected(hasApiKey());
    prevModeRef.current = loadSettings().mode;
  }, []);

  const handleSetupComplete = () => {
    setConnected(true);
    setSettingsVersion((v) => v + 1);
  };

  const handleSettingsChange = () => {
    const currentMode = loadSettings().mode;
    if (prevModeRef.current !== null && prevModeRef.current !== currentMode) {
      const label = currentMode === "explorer" ? "Explorer" : "Assistant";
      setModeNotices((prev) => [...prev, `Switched to ${label} mode`]);
    }
    prevModeRef.current = currentMode;
    setSettingsVersion((v) => v + 1);
  };

  const getApiKey = useCallback(() => loadSettings().apiKey || undefined, []);
  const getModel = useCallback(() => loadSettings().model || undefined, []);
  const getMode = useCallback(() => loadSettings().mode, []);

  const handleIngestComplete = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["am", "episodes"] });
    queryClient.invalidateQueries({ queryKey: ["am", "stats"] });
  }, [queryClient]);

  // Re-create adapter when settings change
  const adapter = useMemo(() => {
    // settingsVersion dependency forces re-creation
    void settingsVersion;
    if (!connected) return mockAdapter;
    return createAMAdapter({
      getApiKey,
      getModel,
      getMode,
    });
  }, [connected, settingsVersion, getApiKey, getModel, getMode]);

  const runtime = useLocalRuntime(adapter);

  // First-launch: show setup card
  if (!connected) {
    return (
      <div
        className="h-dvh"
        style={{ background: "var(--color-bg)" }}
      >
        <SetupCard onComplete={handleSetupComplete} />
      </div>
    );
  }

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <main
        className="relative grid h-dvh"
        style={{
          gridTemplateAreas: `"header header" "chat sidebar"`,
          gridTemplateRows: "var(--header-height) 1fr",
          gridTemplateColumns: "1fr auto",
          background: "var(--color-bg)",
        }}
      >
        <div style={{ gridArea: "header" }}>
          <SettingsBar
            onSettingsChange={handleSettingsChange}
            onIngestComplete={handleIngestComplete}
          />
        </div>
        <div style={{ gridArea: "chat", overflow: "hidden" }}>
          <ChatThread modeNotices={modeNotices} />
        </div>
        <div style={{ gridArea: "sidebar", overflow: "hidden" }}>
          <Sidebar
            onUploadClick={() => setShowUpload(true)}
          />
        </div>
      </main>

      <UploadModal
        open={showUpload}
        onClose={() => setShowUpload(false)}
        onIngestComplete={handleIngestComplete}
      />
    </AssistantRuntimeProvider>
  );
}
