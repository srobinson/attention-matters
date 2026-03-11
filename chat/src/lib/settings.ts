/**
 * Settings store using localStorage.
 * Provides reactive settings for API key, model, and chat mode.
 */

export interface Settings {
  apiKey: string;
  model: string;
  mode: "explorer" | "assistant";
}

const STORAGE_KEY = "am-chat-settings";

const DEFAULT_SETTINGS: Settings = {
  apiKey: "",
  model: "anthropic/claude-3.5-haiku",
  mode: "explorer",
};

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
