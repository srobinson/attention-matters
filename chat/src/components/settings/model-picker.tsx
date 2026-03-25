"use client";

import { useState, useRef, useEffect, useMemo } from "react";
import { createPortal } from "react-dom";
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
  const [menuStyle, setMenuStyle] = useState<{
    top: number;
    left: number;
    width: number;
    maxHeight: number;
  } | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
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
      const target = e.target as Node;
      if (
        containerRef.current &&
        !containerRef.current.contains(target) &&
        (!menuRef.current || !menuRef.current.contains(target))
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

  useEffect(() => {
    if (!open) return;

    const updatePosition = () => {
      const rect = triggerRef.current?.getBoundingClientRect();
      if (!rect) return;

      const viewportHeight = window.innerHeight;
      const viewportWidth = window.innerWidth;
      const preferredWidth = 320;
      const width = Math.min(
        Math.max(rect.width, preferredWidth),
        viewportWidth - 16
      );
      const left = Math.min(rect.left, viewportWidth - width - 8);
      const spaceBelow = viewportHeight - rect.bottom - 8;
      const spaceAbove = rect.top - 8;
      const openUpward = spaceBelow < 240 && spaceAbove > spaceBelow;
      const maxHeight = Math.max(
        160,
        Math.min(360, openUpward ? spaceAbove - 8 : spaceBelow - 8)
      );

      setMenuStyle({
        left: Math.max(8, left),
        top: openUpward
          ? Math.max(8, rect.top - maxHeight - 8)
          : rect.bottom + 8,
        width,
        maxHeight,
      });
    };

    updatePosition();
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
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
        ref={triggerRef}
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 outline-none transition-all hover:border-[var(--color-salient)]"
        style={{
          borderColor: "var(--color-border)",
          background: "var(--color-surface-raised)",
          color: "var(--color-text-secondary)",
          fontSize: "var(--font-size-xs)",
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
      {open &&
        typeof document !== "undefined" &&
        menuStyle &&
        createPortal(
          <div
            ref={menuRef}
            className="animate-fade-slide-down fixed rounded-xl border"
            style={{
              top: menuStyle.top,
              left: menuStyle.left,
              width: menuStyle.width,
              zIndex: "calc(var(--z-modal) + 10)",
              borderColor: "var(--color-border)",
              background: "var(--color-surface)",
              boxShadow: "var(--shadow-xl)",
            }}
            role="listbox"
            aria-label="Available models"
          >
            <div
              className="flex items-center gap-2 border-b px-3 py-2.5"
              style={{ borderColor: "var(--color-border)" }}
            >
              <Search
                className="h-3.5 w-3.5 flex-shrink-0"
                style={{ color: "var(--color-text-tertiary)" }}
              />
              <input
                ref={inputRef}
                type="text"
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder="Search models..."
                className="w-full bg-transparent outline-none placeholder:opacity-40"
                style={{
                  color: "var(--color-text-primary)",
                  fontSize: "var(--font-size-sm)",
                }}
                aria-label="Filter models"
              />
            </div>

            <div
              className="overflow-y-auto overscroll-contain"
              style={{ maxHeight: menuStyle.maxHeight }}
            >
              {recommended.length > 0 && (
                <div className="px-1 py-1">
                  <div className="flex items-center gap-1 px-2 py-1">
                    <Star
                      className="h-2.5 w-2.5"
                      style={{ color: "var(--color-salient)" }}
                    />
                    <span
                      className="font-semibold uppercase"
                      style={{
                        color: "var(--color-salient)",
                        fontSize: "var(--font-size-micro)",
                        letterSpacing: "var(--tracking-wider)",
                      }}
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

              {grouped.map(([provider, providerModels]) => (
                <div key={provider} className="px-1 py-1">
                  <div className="px-2 py-1">
                    <span
                      className="font-semibold uppercase"
                      style={{
                        color: "var(--color-text-tertiary)",
                        fontSize: "var(--font-size-micro)",
                        letterSpacing: "var(--tracking-wider)",
                      }}
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
          </div>,
          document.body
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
      className="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left transition-all"
      style={{
        background: selected ? "var(--color-surface-raised)" : "transparent",
        color: "var(--color-text-primary)",
        fontSize: "var(--font-size-sm)",
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
      <span className="truncate">{model.name}</span>
      <span
        className="flex flex-shrink-0 items-center gap-2 tabular-nums"
        style={{
          color: "var(--color-text-tertiary)",
          fontSize: "var(--font-size-micro)",
        }}
      >
        <span>{formatContext(model.contextLength)} ctx</span>
        <span>{formatPrice(model.promptPrice)}</span>
      </span>
    </button>
  );
}
