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
  model: "anthropic/claude-haiku-4-5-20251001",
  mode: "explorer",
  agentName: "AM",
};

/** Recommended models pinned at the top of the picker. */
export const RECOMMENDED_MODEL_IDS = [
  "anthropic/claude-haiku-4-5-20251001",
  "anthropic/claude-sonnet-4-20250514",
  "anthropic/claude-opus-4-20250514",
  "openai/gpt-4o",
  "google/gemini-2.5-flash",
] as const;

/** Fallback list when the OpenRouter API is unreachable. */
export const FALLBACK_MODELS = [
  { id: "anthropic/claude-haiku-4-5-20251001", name: "Claude Haiku 4.5", provider: "anthropic", contextLength: 200000, promptPrice: 0.0000008, completionPrice: 0.000004 },
  { id: "anthropic/claude-sonnet-4-20250514", name: "Claude Sonnet 4", provider: "anthropic", contextLength: 200000, promptPrice: 0.000003, completionPrice: 0.000015 },
  { id: "anthropic/claude-opus-4-20250514", name: "Claude Opus 4", provider: "anthropic", contextLength: 200000, promptPrice: 0.000015, completionPrice: 0.000075 },
  { id: "openai/gpt-4o", name: "GPT-4o", provider: "openai", contextLength: 128000, promptPrice: 0.0000025, completionPrice: 0.00001 },
  { id: "google/gemini-2.5-flash", name: "Gemini 2.5 Flash", provider: "google", contextLength: 1000000, promptPrice: 0.00000015, completionPrice: 0.0000006 },
] as const;

/**
 * Legacy alias for components that still reference CURATED_MODELS.
 * Maps fallback models to the old { id, label } shape.
 */
export const CURATED_MODELS = FALLBACK_MODELS.map((m) => ({
  id: m.id,
  label: m.name,
}));

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
