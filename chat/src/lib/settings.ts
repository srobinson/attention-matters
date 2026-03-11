/**
 * Settings store using localStorage.
 * Provides reactive settings for API key, model, chat mode, and agent name.
 */

export interface Settings {
  apiKey: string;
  model: string;
  mode: "explorer" | "assistant";
  agentName: string;
}

const STORAGE_KEY = "am-chat-settings";

export const DEFAULT_SETTINGS: Settings = {
  apiKey: "",
  model: "anthropic/claude-3.5-haiku",
  mode: "explorer",
  agentName: "AM",
};

export const CURATED_MODELS = [
  { id: "anthropic/claude-3.5-haiku", label: "Claude 3.5 Haiku" },
  { id: "anthropic/claude-sonnet-4-20250514", label: "Claude Sonnet 4" },
  { id: "openai/gpt-4o", label: "GPT-4o" },
  { id: "google/gemini-2.0-flash", label: "Gemini 2.0 Flash" },
] as const;

export function loadSettings(): Settings {
  if (typeof window === "undefined") return DEFAULT_SETTINGS;
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (!stored) return DEFAULT_SETTINGS;
    const parsed = JSON.parse(stored) as Partial<Settings>;
    return { ...DEFAULT_SETTINGS, ...parsed };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

export function saveSettings(settings: Settings): void {
  if (typeof window === "undefined") return;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
}

export function hasApiKey(): boolean {
  return loadSettings().apiKey.length > 0;
}
