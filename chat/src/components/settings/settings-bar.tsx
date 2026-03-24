"use client";

import { useState, useEffect, useRef, useCallback } from "react";
import { Database, PanelRightOpen, Settings2 } from "lucide-react";
import {
  type Settings,
  loadSettings,
  saveSettings,
} from "@/lib/settings";
import { UploadButton } from "@/components/upload/upload-button";
import { UploadModal } from "@/components/upload/upload-modal";
import { ModelPicker } from "./model-picker";

interface SettingsBarProps {
  onSettingsChange: () => void;
  onIngestComplete?: () => void;
  onMemoryToggle?: () => void;
  mobileMemoryOpen?: boolean;
}

/**
 * Header settings bar with model selector, mode toggle, and settings access.
 * Replaces the plain Header when API key is configured.
 */
export function SettingsBar({
  onSettingsChange,
  onIngestComplete,
  onMemoryToggle,
  mobileMemoryOpen = false,
}: SettingsBarProps) {
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [showDrawer, setShowDrawer] = useState(false);
  const [showUpload, setShowUpload] = useState(false);
  const [modelChangeHint, setModelChangeHint] = useState(false);
  const hintTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (hintTimerRef.current !== null) clearTimeout(hintTimerRef.current);
    };
  }, []);

  const showModelHint = useCallback(() => {
    setModelChangeHint(true);
    if (hintTimerRef.current !== null) clearTimeout(hintTimerRef.current);
    hintTimerRef.current = setTimeout(() => setModelChangeHint(false), 3000);
  }, []);

  const updateSetting = <K extends keyof Settings>(
    key: K,
    value: Settings[K]
  ) => {
    const updated = { ...settings, [key]: value };
    setSettings(updated);
    saveSettings(updated);
    if (key === "model") showModelHint();
    onSettingsChange();
  };

  return (
    <>
      <header
        className="flex items-center justify-between border-b px-3 sm:px-4"
        style={{
          height: "var(--header-height)",
          borderColor: "var(--color-border)",
          background: "color-mix(in srgb, var(--color-surface) 92%, transparent)",
          backdropFilter: "blur(18px)",
        }}
      >
        <div className="flex min-w-0 items-center gap-2.5 sm:gap-3">
          {onMemoryToggle && (
            <button
              onClick={onMemoryToggle}
              className="flex h-10 w-10 items-center justify-center rounded-xl border transition-all lg:hidden"
              style={{
                borderColor: mobileMemoryOpen
                  ? "var(--color-salient)"
                  : "var(--color-border)",
                background: mobileMemoryOpen
                  ? "var(--color-salient-glow)"
                  : "var(--color-surface-raised)",
                color: mobileMemoryOpen
                  ? "var(--color-salient)"
                  : "var(--color-text-secondary)",
              }}
              aria-label={mobileMemoryOpen ? "Close memory explorer" : "Open memory explorer"}
              aria-pressed={mobileMemoryOpen}
            >
              <PanelRightOpen className="h-4 w-4" />
            </button>
          )}

          <div className="min-w-0">
            <div
              className="font-semibold uppercase"
              style={{
                color: "var(--color-salient)",
                fontSize: "var(--font-size-micro)",
                letterSpacing: "0.14em",
              }}
            >
              Memory Console
            </div>
            <h1
              className="truncate font-semibold"
              style={{
                color: "var(--color-text-primary)",
                fontSize: "var(--font-size-sm)",
                letterSpacing: "var(--tracking-tight)",
              }}
            >
              {settings.agentName || "AM"}{" "}
              <span
                style={{
                  color: "var(--color-text-tertiary)",
                  fontWeight: "var(--font-weight-normal)",
                }}
              >
                Chat
              </span>
            </h1>
          </div>

          {/* Model selector */}
          <div className="hidden items-center gap-1.5 sm:flex">
            <ModelPicker
              value={settings.model}
              onChange={(id) => updateSetting("model", id)}
              compact
            />
            {modelChangeHint && (
              <span
                className="animate-pulse"
                style={{
                  color: "var(--color-salient)",
                  fontSize: "var(--font-size-micro)",
                }}
              >
                Takes effect on next message
              </span>
            )}
          </div>

          {/* Mode toggle */}
          <div
            className="hidden items-center gap-0.5 rounded-lg border p-0.5 sm:flex"
            style={{
              borderColor: "var(--color-border-subtle)",
              background: "var(--color-surface-raised)",
            }}
          >
            <ModeButton
              active={settings.mode === "explorer"}
              onClick={() => updateSetting("mode", "explorer")}
              label="Explorer"
            />
            <ModeButton
              active={settings.mode === "assistant"}
              onClick={() => updateSetting("mode", "assistant")}
              label="Assistant"
            />
          </div>
        </div>

        <div className="flex items-center gap-2">
          {/* Connection indicator */}
          <div
            className="hidden items-center gap-2 rounded-full border px-2.5 py-1 sm:flex"
            style={{
              borderColor: "var(--color-border)",
              background: "var(--color-surface-raised)",
            }}
          >
            <div
              className="h-1.5 w-1.5 rounded-full"
              style={{ background: "var(--color-novel)" }}
            />
            <span
              style={{
                color: "var(--color-text-secondary)",
                fontSize: "var(--font-size-xs)",
              }}
            >
              Session live
            </span>
          </div>

          <div
            className="hidden items-center gap-1.5 rounded-full border px-2.5 py-1 md:flex"
            style={{
              borderColor: "var(--color-border)",
              background: "var(--color-surface-raised)",
              color: "var(--color-text-secondary)",
              fontSize: "var(--font-size-xs)",
            }}
          >
            <Database className="h-3.5 w-3.5" />
            Memory aware
          </div>

          {/* Upload button */}
          <UploadButton onClick={() => setShowUpload(true)} />

          {/* Gear icon */}
          <button
            onClick={() => setShowDrawer(!showDrawer)}
            className="flex h-10 w-10 items-center justify-center rounded-xl transition-all hover:bg-[var(--color-surface-raised)]"
            style={{ color: "var(--color-text-secondary)" }}
            aria-label="Open settings"
          >
            <Settings2 className="h-4 w-4" />
          </button>
        </div>
      </header>

      {/* Settings drawer */}
      {showDrawer && (
        <SettingsDrawer
          settings={settings}
          onUpdate={updateSetting}
          onClose={() => setShowDrawer(false)}
          modelChangeHint={modelChangeHint}
        />
      )}

      {/* Upload modal */}
      <UploadModal
        open={showUpload}
        onClose={() => setShowUpload(false)}
        onIngestComplete={onIngestComplete}
      />
    </>
  );
}

function ModeButton({
  active,
  onClick,
  label,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className="rounded-md px-2.5 py-1 font-medium transition-all"
      style={{
        background: active ? "var(--color-surface)" : "transparent",
        color: active
          ? "var(--color-text-primary)"
          : "var(--color-text-tertiary)",
        fontSize: "var(--font-size-xs)",
        boxShadow: active ? "var(--shadow-sm)" : "none",
      }}
    >
      {label}
    </button>
  );
}

function SettingsDrawer({
  settings,
  onUpdate,
  onClose,
  modelChangeHint,
}: {
  settings: Settings;
  onUpdate: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
  onClose: () => void;
  modelChangeHint: boolean;
}) {
  return (
    <div
      className="animate-fade-slide-down absolute right-0 top-[var(--header-height)] z-50 w-80 rounded-bl-xl border-b border-l p-5"
      style={{
        borderColor: "var(--color-border)",
        background: "var(--color-surface)",
        boxShadow: "var(--shadow-lg)",
      }}
    >
      <div className="flex flex-col gap-5">
        <div className="flex items-center justify-between">
          <h2
            className="font-semibold"
            style={{
              color: "var(--color-text-primary)",
              fontSize: "var(--font-size-sm)",
            }}
          >
            Settings
          </h2>
          <button
            onClick={onClose}
            className="rounded-md px-2 py-1 transition-colors hover:bg-[var(--color-surface-raised)]"
            style={{
              color: "var(--color-text-secondary)",
              fontSize: "var(--font-size-xs)",
            }}
          >
            Close
          </button>
        </div>

        {/* API Key */}
        <SettingsField label="API Key">
          <input
            type="password"
            value={settings.apiKey}
            onChange={(e) => onUpdate("apiKey", e.target.value)}
            className="w-full rounded-lg border px-3 py-2 outline-none transition-colors focus:border-[var(--color-salient)]"
            style={{
              borderColor: "var(--color-border)",
              background: "var(--color-surface-raised)",
              color: "var(--color-text-primary)",
              fontSize: "var(--font-size-sm)",
            }}
          />
          <p
            style={{
              color: "var(--color-text-tertiary)",
              fontSize: "var(--font-size-micro)",
              lineHeight: "var(--line-height-relaxed)",
            }}
          >
            Stored in your browser. Sent to the AM server for LLM requests,
            forwarded to OpenRouter, never persisted on the server.
          </p>
        </SettingsField>

        {/* Agent Name */}
        <SettingsField label="Agent Name">
          <input
            type="text"
            value={settings.agentName}
            onChange={(e) => onUpdate("agentName", e.target.value)}
            placeholder="AM"
            className="w-full rounded-lg border px-3 py-2 outline-none transition-colors focus:border-[var(--color-salient)]"
            style={{
              borderColor: "var(--color-border)",
              background: "var(--color-surface-raised)",
              color: "var(--color-text-primary)",
              fontSize: "var(--font-size-sm)",
            }}
          />
        </SettingsField>

        {/* Model (mobile) */}
        <SettingsField label="Model">
          <ModelPicker
            value={settings.model}
            onChange={(id) => onUpdate("model", id)}
          />
          {modelChangeHint && (
            <span
              className="animate-pulse"
              style={{
                color: "var(--color-salient)",
                fontSize: "var(--font-size-micro)",
              }}
            >
              Takes effect on next message
            </span>
          )}
        </SettingsField>

        {/* Mode (mobile) */}
        <SettingsField label="Chat Mode">
          <div className="flex gap-1">
            <ModeButton
              active={settings.mode === "explorer"}
              onClick={() => onUpdate("mode", "explorer")}
              label="Explorer"
            />
            <ModeButton
              active={settings.mode === "assistant"}
              onClick={() => onUpdate("mode", "assistant")}
              label="Assistant"
            />
          </div>
        </SettingsField>
      </div>
    </div>
  );
}

function SettingsField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label
        className="font-medium uppercase"
        style={{
          color: "var(--color-text-tertiary)",
          fontSize: "var(--font-size-micro)",
          letterSpacing: "var(--tracking-wider)",
        }}
      >
        {label}
      </label>
      {children}
    </div>
  );
}
