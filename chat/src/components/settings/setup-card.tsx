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

  const handleSubmit = async (e: React.FormEvent<HTMLFormElement>) => {
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
        className="animate-fade-in w-full max-w-md rounded-2xl border p-8"
        style={{
          borderColor: "var(--color-border)",
          background: "var(--color-surface)",
          boxShadow: "var(--shadow-xl)",
        }}
      >
        <div className="mb-8 flex flex-col items-center gap-4">
          <div
            className="flex h-14 w-14 items-center justify-center rounded-2xl"
            style={{
              background: "var(--color-salient-glow)",
              boxShadow: "var(--shadow-glow-gold)",
            }}
          >
            <KeyRound
              className="h-6 w-6"
              style={{ color: "var(--color-salient)" }}
            />
          </div>
          <h2
            className="text-center font-semibold"
            style={{
              color: "var(--color-text-primary)",
              fontSize: "var(--font-size-xl)",
              letterSpacing: "var(--tracking-tight)",
              lineHeight: "var(--line-height-tight)",
            }}
          >
            Connect a model to start chatting with your memory
          </h2>
          <p
            className="text-center"
            style={{
              color: "var(--color-text-secondary)",
              fontSize: "var(--font-size-sm)",
              lineHeight: "var(--line-height-relaxed)",
            }}
          >
            Single API key, access to Claude, GPT, Gemini, and more. Your key
            is stored in your browser and sent to the AM server for LLM
            requests. The server forwards it to OpenRouter and does not persist
            it.
          </p>
        </div>

        <div className="flex flex-col gap-5">
          {/* API Key */}
          <div className="flex flex-col gap-2">
            <label
              htmlFor="api-key"
              className="font-medium uppercase"
              style={{
                color: "var(--color-text-tertiary)",
                fontSize: "var(--font-size-micro)",
                letterSpacing: "var(--tracking-wider)",
              }}
            >
              OpenRouter API Key
            </label>
            <input
              id="api-key"
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder="sk-or-..."
              className="rounded-lg border px-3 py-2.5 outline-none transition-colors focus:border-[var(--color-salient)]"
              style={{
                borderColor: "var(--color-border)",
                background: "var(--color-surface-raised)",
                color: "var(--color-text-primary)",
                fontSize: "var(--font-size-base)",
              }}
              autoFocus
            />
          </div>

          {/* Model */}
          <div className="flex flex-col gap-2">
            <label
              htmlFor="model"
              className="font-medium uppercase"
              style={{
                color: "var(--color-text-tertiary)",
                fontSize: "var(--font-size-micro)",
                letterSpacing: "var(--tracking-wider)",
              }}
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
            <p
              style={{
                color: "var(--color-error)",
                fontSize: "var(--font-size-sm)",
              }}
            >
              {error}
            </p>
          )}

          {/* Submit */}
          <button
            type="submit"
            disabled={validating || !apiKey.trim()}
            className="flex items-center justify-center gap-2 rounded-lg px-4 py-2.5 font-medium transition-all hover:brightness-110 disabled:opacity-50"
            style={{
              background: "var(--color-salient)",
              color: "var(--color-bg)",
              fontSize: "var(--font-size-base)",
              boxShadow: "var(--shadow-md)",
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
