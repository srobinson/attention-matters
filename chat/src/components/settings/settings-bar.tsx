"use client";

import { useState, useEffect } from "react";
import { Settings2 } from "lucide-react";
import {
  type Settings,
  CURATED_MODELS,
  loadSettings,
  saveSettings,
} from "@/lib/settings";
import { UploadButton } from "@/components/upload/upload-button";
import { UploadModal } from "@/components/upload/upload-modal";

interface SettingsBarProps {
  onSettingsChange: () => void;
  onIngestComplete?: () => void;
}

/**
 * Header settings bar with model selector, mode toggle, and settings access.
 * Replaces the plain Header when API key is configured.
 */
export function SettingsBar({ onSettingsChange, onIngestComplete }: SettingsBarProps) {
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [showDrawer, setShowDrawer] = useState(false);
  const [showUpload, setShowUpload] = useState(false);

  // Reload settings when component mounts (SSR safety)
  useEffect(() => {
    setSettings(loadSettings());
  }, []);

  const updateSetting = <K extends keyof Settings>(
    key: K,
    value: Settings[K]
  ) => {
    const updated = { ...settings, [key]: value };
    setSettings(updated);
    saveSettings(updated);
    onSettingsChange();
  };

  return (
    <>
      <header
        className="flex items-center justify-between border-b px-4"
        style={{
          height: "var(--header-height)",
          borderColor: "var(--color-border)",
          background: "var(--color-surface)",
        }}
      >
        <div className="flex items-center gap-3">
          <h1
            className="text-sm font-medium"
            style={{ color: "var(--color-text-primary)" }}
          >
            {settings.agentName || "AM"} Chat
          </h1>

          {/* Model selector */}
          <select
            value={settings.model}
            onChange={(e) => updateSetting("model", e.target.value)}
            className="hidden rounded border px-2 py-1 text-xs outline-none sm:block"
            style={{
              borderColor: "var(--color-border)",
              background: "var(--color-surface-raised)",
              color: "var(--color-text-secondary)",
            }}
            aria-label="Select model"
          >
            {CURATED_MODELS.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
          </select>

          {/* Mode toggle */}
          <div
            className="hidden items-center gap-1 rounded border p-0.5 sm:flex"
            style={{
              borderColor: "var(--color-border)",
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
          <span
            className="hidden text-xs sm:inline"
            style={{ color: "var(--color-novel)" }}
          >
            Connected
          </span>
          <div
            className="h-2 w-2 rounded-full"
            style={{ background: "var(--color-novel)" }}
          />

          {/* Upload button */}
          <UploadButton onClick={() => setShowUpload(true)} />

          {/* Gear icon */}
          <button
            onClick={() => setShowDrawer(!showDrawer)}
            className="flex h-8 w-8 items-center justify-center rounded-md transition-colors hover:opacity-80"
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
      className="rounded px-2 py-0.5 text-[11px] font-medium transition-colors"
      style={{
        background: active ? "var(--color-surface)" : "transparent",
        color: active
          ? "var(--color-text-primary)"
          : "var(--color-text-secondary)",
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
}: {
  settings: Settings;
  onUpdate: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
  onClose: () => void;
}) {
  return (
    <div
      className="absolute right-0 top-[var(--header-height)] z-50 w-80 rounded-bl-lg border-b border-l p-4"
      style={{
        borderColor: "var(--color-border)",
        background: "var(--color-surface)",
      }}
    >
      <div className="flex flex-col gap-4">
        <div className="flex items-center justify-between">
          <h2
            className="text-sm font-medium"
            style={{ color: "var(--color-text-primary)" }}
          >
            Settings
          </h2>
          <button
            onClick={onClose}
            className="text-xs hover:opacity-80"
            style={{ color: "var(--color-text-secondary)" }}
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
            className="w-full rounded border px-2 py-1.5 text-xs outline-none"
            style={{
              borderColor: "var(--color-border)",
              background: "var(--color-surface-raised)",
              color: "var(--color-text-primary)",
            }}
          />
        </SettingsField>

        {/* Agent Name */}
        <SettingsField label="Agent Name">
          <input
            type="text"
            value={settings.agentName}
            onChange={(e) => onUpdate("agentName", e.target.value)}
            placeholder="AM"
            className="w-full rounded border px-2 py-1.5 text-xs outline-none"
            style={{
              borderColor: "var(--color-border)",
              background: "var(--color-surface-raised)",
              color: "var(--color-text-primary)",
            }}
          />
        </SettingsField>

        {/* Model (mobile) */}
        <SettingsField label="Model">
          <select
            value={settings.model}
            onChange={(e) => onUpdate("model", e.target.value)}
            className="w-full rounded border px-2 py-1.5 text-xs outline-none"
            style={{
              borderColor: "var(--color-border)",
              background: "var(--color-surface-raised)",
              color: "var(--color-text-primary)",
            }}
          >
            {CURATED_MODELS.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
          </select>
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
    <div className="flex flex-col gap-1">
      <label
        className="text-[11px] font-medium"
        style={{ color: "var(--color-text-secondary)" }}
      >
        {label}
      </label>
      {children}
    </div>
  );
}
