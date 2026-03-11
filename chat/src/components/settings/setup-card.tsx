"use client";

import { useState } from "react";
import { KeyRound, Loader2 } from "lucide-react";
import {
  type Settings,
  DEFAULT_SETTINGS,
  saveSettings,
} from "@/lib/settings";
import { amHealth } from "@/lib/am-client";
import { ModelPicker } from "./model-picker";

interface SetupCardProps {
  onComplete: () => void;
}

/**
 * First-launch setup card. Centered overlay prompting for API key
 * configuration before chat activates.
 */
export function SetupCard({ onComplete }: SetupCardProps) {
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState(DEFAULT_SETTINGS.model);
  const [validating, setValidating] = useState(false);
  const [error, setError] = useState("");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!apiKey.trim()) {
      setError("API key is required");
      return;
    }

    setValidating(true);
    setError("");

    try {
      // Validate AM backend is reachable
      await amHealth();

      // Save settings and transition to chat
      const settings: Settings = {
        apiKey: apiKey.trim(),
        model,
        mode: DEFAULT_SETTINGS.mode,
        agentName: DEFAULT_SETTINGS.agentName,
      };
      saveSettings(settings);
      onComplete();
    } catch {
      setError(
        "Could not connect to AM backend. Make sure am serve --http 3001 is running."
      );
    } finally {
      setValidating(false);
    }
  };

  return (
    <div
      className="flex h-full items-center justify-center px-4"
      style={{ background: "var(--color-bg)" }}
    >
      <form
        onSubmit={handleSubmit}
        className="w-full max-w-md rounded-xl border p-6"
        style={{
          borderColor: "var(--color-border)",
          background: "var(--color-surface)",
        }}
      >
        <div className="mb-6 flex flex-col items-center gap-3">
          <div
            className="flex h-10 w-10 items-center justify-center rounded-lg"
            style={{ background: "var(--color-surface-raised)" }}
          >
            <KeyRound
              className="h-5 w-5"
              style={{ color: "var(--color-salient)" }}
            />
          </div>
          <h2
            className="text-center text-lg font-semibold"
            style={{ color: "var(--color-text-primary)" }}
          >
            Connect a model to start chatting with your memory
          </h2>
          <p
            className="text-center text-xs leading-relaxed"
            style={{ color: "var(--color-text-secondary)" }}
          >
            Single API key, access to Claude, GPT, Gemini, and more. Your key
            is stored in your browser and sent to the AM server for LLM
            requests. The server forwards it to OpenRouter and does not persist
            it.
          </p>
        </div>

        <div className="flex flex-col gap-4">
          {/* API Key */}
          <div className="flex flex-col gap-1.5">
            <label
              htmlFor="api-key"
              className="text-xs font-medium"
              style={{ color: "var(--color-text-primary)" }}
            >
              OpenRouter API Key
            </label>
            <input
              id="api-key"
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder="sk-or-..."
              className="rounded-md border px-3 py-2 text-sm outline-none transition-colors focus:ring-1"
              style={{
                borderColor: "var(--color-border)",
                background: "var(--color-surface-raised)",
                color: "var(--color-text-primary)",
              }}
              autoFocus
            />
          </div>

          {/* Model */}
          <div className="flex flex-col gap-1.5">
            <label
              htmlFor="model"
              className="text-xs font-medium"
              style={{ color: "var(--color-text-primary)" }}
            >
              Model
            </label>
            <ModelPicker
              value={model}
              onChange={setModel}
            />
          </div>

          {/* Error */}
          {error && (
            <p className="text-xs" style={{ color: "#ef4444" }}>
              {error}
            </p>
          )}

          {/* Submit */}
          <button
            type="submit"
            disabled={validating || !apiKey.trim()}
            className="flex items-center justify-center gap-2 rounded-md px-4 py-2 text-sm font-medium transition-colors disabled:opacity-50"
            style={{
              background: "var(--color-salient)",
              color: "var(--color-bg)",
            }}
          >
            {validating ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" />
                Connecting...
              </>
            ) : (
              "Connect"
            )}
          </button>
        </div>
      </form>
    </div>
  );
}
