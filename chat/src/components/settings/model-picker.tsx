"use client";

import { useState, useRef, useEffect, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { ChevronDown, Search, Star, Loader2 } from "lucide-react";
import {
  RECOMMENDED_MODEL_IDS,
  FALLBACK_MODELS,
} from "@/lib/settings";

interface OpenRouterModel {
  id: string;
  name: string;
  provider: string;
  contextLength: number;
  promptPrice: number;
  completionPrice: number;
}

interface ModelPickerProps {
  value: string;
  onChange: (modelId: string) => void;
  compact?: boolean;
}

async function fetchModels(): Promise<OpenRouterModel[]> {
  const res = await fetch("/api/models");
  if (!res.ok) throw new Error("Failed to fetch models");
  const json = await res.json();
  return json.models;
}

function formatContext(length: number): string {
  if (length >= 1000000) return `${(length / 1000000).toFixed(1)}M`;
  if (length >= 1000) return `${Math.round(length / 1000)}K`;
  return String(length);
}

function formatPrice(price: number): string {
  if (price === 0) return "Free";
  // Price per million tokens
  const perMillion = price * 1000000;
  if (perMillion < 0.01) return "<$0.01/M";
  if (perMillion < 1) return `$${perMillion.toFixed(2)}/M`;
  return `$${perMillion.toFixed(1)}/M`;
}

/**
 * Searchable model picker with dynamic OpenRouter model list.
 * Recommended models pinned at top, full list grouped by provider.
 * Falls back to hardcoded list on API failure.
 */
export function ModelPicker({ value, onChange, compact }: ModelPickerProps) {
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const { data: models, isLoading } = useQuery<OpenRouterModel[]>({
    queryKey: ["openrouter", "models"],
    queryFn: fetchModels,
    staleTime: 3600000, // 1 hour
    retry: 1,
  });

  // Use fetched models or fallback
  const allModels = useMemo(
    () => (models && models.length > 0 ? models : [...FALLBACK_MODELS]),
    [models]
  );

  // Find current model display name
  const currentModel = allModels.find((m) => m.id === value);
  const displayName = currentModel?.name ?? value.split("/").pop() ?? value;

  // Filter and group
  const { recommended, grouped } = useMemo(() => {
    const term = filter.toLowerCase();
    const filtered = term
      ? allModels.filter(
          (m) =>
            m.name.toLowerCase().includes(term) ||
            m.id.toLowerCase().includes(term) ||
            m.provider.toLowerCase().includes(term)
        )
      : allModels;

    const rec = filtered.filter((m) =>
      (RECOMMENDED_MODEL_IDS as readonly string[]).includes(m.id)
    );

    const rest = filtered.filter(
      (m) => !(RECOMMENDED_MODEL_IDS as readonly string[]).includes(m.id)
    );

    const byProvider = new Map<string, OpenRouterModel[]>();
    for (const m of rest) {
      const existing = byProvider.get(m.provider);
      if (existing) {
        existing.push(m);
      } else {
        byProvider.set(m.provider, [m]);
      }
    }

    // Sort providers alphabetically
    const sortedProviders = [...byProvider.entries()].sort((a, b) =>
      a[0].localeCompare(b[0])
    );

    return { recommended: rec, grouped: sortedProviders };
  }, [allModels, filter]);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    const handleClick = (e: MouseEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node)
      ) {
        setOpen(false);
        setFilter("");
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open]);

  // Focus search when opened
  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        setFilter("");
      }
    };
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [open]);

  const selectModel = (id: string) => {
    onChange(id);
    setOpen(false);
    setFilter("");
  };

  return (
    <div ref={containerRef} className="relative">
      {/* Trigger button */}
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1.5 rounded border px-2 py-1 text-xs outline-none transition-colors hover:opacity-80"
        style={{
          borderColor: "var(--color-border)",
          background: "var(--color-surface-raised)",
          color: "var(--color-text-secondary)",
          maxWidth: compact ? "160px" : "240px",
        }}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label="Select model"
      >
        <span className="truncate">{displayName}</span>
        {isLoading ? (
          <Loader2 className="h-3 w-3 flex-shrink-0 animate-spin" />
        ) : (
          <ChevronDown className="h-3 w-3 flex-shrink-0" />
        )}
      </button>

      {/* Dropdown */}
      {open && (
        <div
          className="absolute left-0 top-full z-50 mt-1 w-80 rounded-lg border shadow-xl"
          style={{
            borderColor: "var(--color-border)",
            background: "var(--color-surface)",
          }}
          role="listbox"
          aria-label="Available models"
        >
          {/* Search */}
          <div
            className="flex items-center gap-2 border-b px-3 py-2"
            style={{ borderColor: "var(--color-border)" }}
          >
            <Search
              className="h-3.5 w-3.5 flex-shrink-0"
              style={{ color: "var(--color-text-secondary)" }}
            />
            <input
              ref={inputRef}
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Search models..."
              className="w-full bg-transparent text-xs outline-none placeholder:opacity-50"
              style={{ color: "var(--color-text-primary)" }}
              aria-label="Filter models"
            />
          </div>

          {/* Model list */}
          <div className="max-h-72 overflow-y-auto">
            {/* Recommended */}
            {recommended.length > 0 && (
              <div className="px-1 py-1">
                <div
                  className="flex items-center gap-1 px-2 py-1"
                >
                  <Star
                    className="h-2.5 w-2.5"
                    style={{ color: "var(--color-salient)" }}
                  />
                  <span
                    className="text-[10px] font-medium uppercase tracking-wider"
                    style={{ color: "var(--color-salient)" }}
                  >
                    Recommended
                  </span>
                </div>
                {recommended.map((m) => (
                  <ModelOption
                    key={m.id}
                    model={m}
                    selected={m.id === value}
                    onClick={() => selectModel(m.id)}
                  />
                ))}
              </div>
            )}

            {/* Grouped by provider */}
            {grouped.map(([provider, providerModels]) => (
              <div key={provider} className="px-1 py-1">
                <div className="px-2 py-1">
                  <span
                    className="text-[10px] font-medium uppercase tracking-wider"
                    style={{ color: "var(--color-text-secondary)" }}
                  >
                    {provider}
                  </span>
                </div>
                {providerModels.map((m) => (
                  <ModelOption
                    key={m.id}
                    model={m}
                    selected={m.id === value}
                    onClick={() => selectModel(m.id)}
                  />
                ))}
              </div>
            ))}

            {recommended.length === 0 && grouped.length === 0 && (
              <div className="px-3 py-4 text-center">
                <span
                  className="text-xs"
                  style={{ color: "var(--color-text-secondary)" }}
                >
                  No models match your search
                </span>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function ModelOption({
  model,
  selected,
  onClick,
}: {
  model: OpenRouterModel;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="flex w-full items-center justify-between rounded px-2 py-1.5 text-left transition-colors"
      style={{
        background: selected ? "var(--color-surface-raised)" : "transparent",
        color: "var(--color-text-primary)",
      }}
      onMouseEnter={(e) => {
        if (!selected)
          e.currentTarget.style.background = "var(--color-surface-raised)";
      }}
      onMouseLeave={(e) => {
        if (!selected) e.currentTarget.style.background = "transparent";
      }}
      role="option"
      aria-selected={selected}
    >
      <span className="truncate text-xs">{model.name}</span>
      <span
        className="flex flex-shrink-0 items-center gap-2 text-[10px] tabular-nums"
        style={{ color: "var(--color-text-secondary)" }}
      >
        <span>{formatContext(model.contextLength)} ctx</span>
        <span>{formatPrice(model.promptPrice)}</span>
      </span>
    </button>
  );
}
