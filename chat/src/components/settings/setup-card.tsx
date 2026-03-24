"use client";

import { useState } from "react";
import { KeyRound, Loader2, Orbit, ShieldCheck, Sparkles } from "lucide-react";
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
      className="relative flex h-full items-center justify-center overflow-hidden px-4 py-8"
      style={{ background: "transparent" }}
    >
      <div
        className="pointer-events-none absolute inset-0 opacity-90"
        style={{
          background:
            "radial-gradient(circle at top, var(--color-salient-glow), transparent 22rem), radial-gradient(circle at right 18% bottom 18%, rgba(96, 180, 240, 0.12), transparent 18rem)",
        }}
      />
      <form
        onSubmit={handleSubmit}
        className="animate-fade-in relative w-full max-w-2xl overflow-hidden rounded-[2rem] border p-6 sm:p-8"
        style={{
          borderColor: "var(--color-border)",
          background: "color-mix(in srgb, var(--color-surface) 94%, transparent)",
          boxShadow: "var(--shadow-xl)",
          backdropFilter: "blur(24px)",
        }}
      >
        <div className="grid gap-8 lg:grid-cols-[1.15fr_0.85fr]">
          <div className="flex flex-col gap-6">
            <div className="flex flex-col gap-4">
              <div
                className="inline-flex w-fit items-center gap-2 rounded-full border px-3 py-1"
                style={{
                  borderColor: "var(--color-border)",
                  background: "var(--color-surface-raised)",
                  color: "var(--color-salient)",
                  fontSize: "var(--font-size-xs)",
                }}
              >
                <Sparkles className="h-3.5 w-3.5" />
                Memory-aware conversation
              </div>
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
              <div className="flex flex-col gap-3">
                <h2
                  className="max-w-xl font-semibold"
                  style={{
                    color: "var(--color-text-primary)",
                    fontSize: "clamp(2rem, 4vw, 3.2rem)",
                    letterSpacing: "var(--tracking-tight)",
                    lineHeight: 1,
                  }}
                >
                  Turn your history into a working memory layer for every conversation.
                </h2>
                <p
                  className="max-w-lg"
                  style={{
                    color: "var(--color-text-secondary)",
                    fontSize: "var(--font-size-base)",
                    lineHeight: "var(--line-height-relaxed)",
                  }}
                >
                  AM keeps track of what matters, recalls relevant context, and
                  surfaces patterns while you chat. Connect a model once, then
                  use the workspace like an external memory system.
                </p>
              </div>
            </div>

            <div className="grid gap-3 sm:grid-cols-3">
              <ValueCard
                icon={Orbit}
                title="Context on tap"
                body="Each response can pull in recalled context and related memory clusters."
              />
              <ValueCard
                icon={Sparkles}
                title="Signals, not clutter"
                body="Pinned memories, latent recalls, and novel connections stay visually distinct."
              />
              <ValueCard
                icon={ShieldCheck}
                title="Your key stays local"
                body="Stored in the browser, forwarded by the AM server, not persisted there."
              />
            </div>
          </div>
          <div
            className="flex flex-col gap-5 rounded-[1.5rem] border p-5 sm:p-6"
            style={{
              borderColor: "var(--color-border)",
              background: "color-mix(in srgb, var(--color-surface-raised) 82%, transparent)",
            }}
          >
            <div className="flex flex-col gap-1.5">
              <span
                className="font-semibold uppercase"
                style={{
                  color: "var(--color-text-tertiary)",
                  fontSize: "var(--font-size-micro)",
                  letterSpacing: "var(--tracking-wider)",
                }}
              >
                Connect your model
              </span>
              <p
                style={{
                  color: "var(--color-text-secondary)",
                  fontSize: "var(--font-size-sm)",
                  lineHeight: "var(--line-height-relaxed)",
                }}
              >
                Start with an OpenRouter key, choose a model, and enter the
                memory workspace.
              </p>
            </div>

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
                className="rounded-xl border px-4 py-3 outline-none transition-colors focus:border-[var(--color-salient)]"
                style={{
                  borderColor: "var(--color-border)",
                  background: "var(--color-surface)",
                  color: "var(--color-text-primary)",
                  fontSize: "var(--font-size-base)",
                }}
                autoFocus
              />
            </div>

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

            <button
              type="submit"
              disabled={validating || !apiKey.trim()}
              className="flex min-h-12 items-center justify-center gap-2 rounded-xl px-4 py-3 font-medium transition-all hover:brightness-110 disabled:opacity-50"
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
                "Enter the workspace"
              )}
            </button>

            <p
              style={{
                color: "var(--color-text-tertiary)",
                fontSize: "var(--font-size-xs)",
                lineHeight: "var(--line-height-relaxed)",
              }}
            >
              Works with Claude, GPT, Gemini, and other OpenRouter-backed models.
            </p>
          </div>
        </div>
      </form>
    </div>
  );
}

function ValueCard({
  icon: Icon,
  title,
  body,
}: {
  icon: typeof Orbit;
  title: string;
  body: string;
}) {
  return (
    <div
      className="rounded-[1.25rem] border p-4"
      style={{
        borderColor: "var(--color-border)",
        background: "color-mix(in srgb, var(--color-surface-raised) 78%, transparent)",
      }}
    >
      <div className="mb-3 flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--color-salient-glow)]">
        <Icon className="h-4 w-4" style={{ color: "var(--color-salient)" }} />
      </div>
      <div className="flex flex-col gap-1.5">
        <h3
          className="font-medium"
          style={{
            color: "var(--color-text-primary)",
            fontSize: "var(--font-size-sm)",
          }}
        >
          {title}
        </h3>
        <p
          style={{
            color: "var(--color-text-secondary)",
            fontSize: "var(--font-size-xs)",
            lineHeight: "var(--line-height-relaxed)",
          }}
        >
          {body}
        </p>
      </div>
    </div>
  );
}
