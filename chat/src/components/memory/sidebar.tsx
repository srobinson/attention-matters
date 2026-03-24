"use client";

import {
  useState,
  useEffect,
  useCallback,
  useRef,
} from "react";
import { Layers, Search, X } from "lucide-react";
import { StatsHeader } from "./stats-header";
import { EpisodeList } from "./episode-list";
import { EpisodeDetail } from "./episode-detail";
import { MemorySearch } from "./memory-search";
import type { Episode } from "@/lib/types";

type SidebarTab = "episodes" | "search";

const STORAGE_KEY = "am_sidebar_tab";
const SIDEBAR_COLLAPSED_WIDTH = 36;
const SIDEBAR_DEFAULT_WIDTH = 280;
const SIDEBAR_MIN_WIDTH = 150;
const SIDEBAR_MAX_WIDTH = 500;

function loadTab(): SidebarTab {
  if (typeof window === "undefined") return "episodes";
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored === "search") return "search";
  return "episodes";
}

interface SidebarProps {
  onUploadClick?: () => void;
  mobileOpen?: boolean;
  onMobileClose?: () => void;
}

/**
 * Collapsible sidebar with icon rail and expandable panel.
 * Collapsed: 32px icon rail with Episodes (stack) and Search (magnifier) icons.
 * Expanded: 280px default, resizable via drag handle (150-500px range).
 * Keyboard: Cmd+\ toggles expansion.
 * Tab state persists in localStorage.
 *
 * Responsive: hidden below 1024px, accessible via toggle.
 */
export function Sidebar({
  onUploadClick,
  mobileOpen = false,
  onMobileClose,
}: SidebarProps) {
  const [expanded, setExpanded] = useState(false);
  const [tab, setTab] = useState<SidebarTab>(loadTab);
  const [width, setWidth] = useState(SIDEBAR_DEFAULT_WIDTH);
  const [selectedEpisode, setSelectedEpisode] = useState<Episode | null>(null);
  const [isResizing, setIsResizing] = useState(false);
  const isDragging = useRef(false);
  const sidebarRef = useRef<HTMLDivElement>(null);

  // Persist tab choice
  useEffect(() => {
    localStorage.setItem(STORAGE_KEY, tab);
  }, [tab]);

  // Cmd+\ keyboard shortcut
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "\\") {
        e.preventDefault();
        setExpanded((prev) => !prev);
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, []);

  const handleTabClick = useCallback(
    (newTab: SidebarTab) => {
      if (expanded && tab === newTab) {
        // Clicking active tab collapses sidebar
        setExpanded(false);
      } else {
        setTab(newTab);
        setExpanded(true);
      }
    },
    [expanded, tab]
  );

  // Drag handle for resizing
  const handleDragStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    isDragging.current = true;
    setIsResizing(true);
    const startX = e.clientX;
    const startWidth = sidebarRef.current?.getBoundingClientRect().width ?? SIDEBAR_DEFAULT_WIDTH;

    const handleMouseMove = (moveEvent: MouseEvent) => {
      if (!isDragging.current) return;
      // Sidebar is on the right, so moving left increases width
      const delta = startX - moveEvent.clientX;
      const newWidth = Math.min(
        SIDEBAR_MAX_WIDTH,
        Math.max(SIDEBAR_MIN_WIDTH, startWidth + delta)
      );
      setWidth(newWidth);
    };

    const handleMouseUp = () => {
      isDragging.current = false;
      setIsResizing(false);
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
  }, []);

  const sidebarWidth = expanded
    ? width + SIDEBAR_COLLAPSED_WIDTH
    : SIDEBAR_COLLAPSED_WIDTH;

  const sidebarContent = (
    <>
      <div
        className="flex flex-shrink-0 flex-col border-b"
        style={{ borderColor: "var(--color-border)" }}
      >
        <div className="flex items-center justify-between gap-3 px-4 py-3 lg:px-3 lg:pt-3 lg:pb-1">
          <div className="flex min-w-0 items-center gap-2">
            <span
              className="font-semibold uppercase"
              style={{
                color: "var(--color-text-secondary)",
                fontSize: "var(--font-size-micro)",
                letterSpacing: "var(--tracking-wider)",
              }}
            >
              Memory Explorer
            </span>
            <span
              style={{
                color: "var(--color-text-tertiary)",
                fontSize: "var(--font-size-micro)",
              }}
            >
              All memories
            </span>
          </div>
          {onMobileClose && (
            <button
              onClick={onMobileClose}
              className="flex h-9 w-9 items-center justify-center rounded-xl border lg:hidden"
              style={{
                borderColor: "var(--color-border)",
                background: "var(--color-surface-raised)",
                color: "var(--color-text-secondary)",
              }}
              aria-label="Close memory explorer"
            >
              <X className="h-4 w-4" />
            </button>
          )}
        </div>
        <div className="flex items-center gap-2 px-4 pb-3 lg:hidden">
          <MobileTabButton
            active={tab === "episodes"}
            icon={<Layers className="h-3.5 w-3.5" />}
            label="Episodes"
            onClick={() => setTab("episodes")}
          />
          <MobileTabButton
            active={tab === "search"}
            icon={<Search className="h-3.5 w-3.5" />}
            label="Search"
            onClick={() => setTab("search")}
          />
        </div>
        <StatsHeader />
      </div>

      <div className="flex min-h-0 w-full flex-1 flex-col overflow-hidden pt-2">
        {tab === "episodes" && selectedEpisode && (
          <EpisodeDetail
            episode={selectedEpisode}
            onBack={() => setSelectedEpisode(null)}
          />
        )}
        {tab === "episodes" && !selectedEpisode && (
          <EpisodeList
            onSelectEpisode={(ep) => {
              setSelectedEpisode(ep);
            }}
            onUploadClick={onUploadClick}
          />
        )}
        {tab === "search" && <MemorySearch />}
      </div>
    </>
  );

  return (
    <>
      {mobileOpen && (
        <div className="fixed inset-0 z-40 lg:hidden">
          <button
            className="absolute inset-0 bg-black/60"
            onClick={onMobileClose}
            aria-label="Close memory explorer"
          />
          <aside
            className="animate-fade-slide-up absolute inset-x-3 bottom-3 top-[calc(var(--header-height)+0.75rem)] overflow-hidden rounded-[1.75rem] border"
            style={{
              borderColor: "var(--color-border)",
              background: "color-mix(in srgb, var(--color-surface) 94%, black)",
              boxShadow: "var(--shadow-xl)",
              backdropFilter: "blur(20px)",
            }}
            aria-label="Memory explorer sidebar"
          >
            <div className="flex h-full min-h-0 flex-col overflow-hidden">
              {sidebarContent}
            </div>
          </aside>
        </div>
      )}

      <aside
        ref={sidebarRef}
        className="relative hidden h-full min-h-0 flex-shrink-0 border-l lg:flex"
        style={{
          width: sidebarWidth,
          borderColor: "var(--color-border)",
          background: "var(--color-surface)",
          transition: isResizing ? "none" : "width var(--transition-spring)",
        }}
        aria-label="Memory explorer sidebar"
      >
        {/* Drag handle (visible only when expanded) */}
        {expanded && (
          <div
            className="absolute left-0 top-0 z-10 h-full w-1 cursor-col-resize hover:opacity-100"
            style={{ background: "transparent" }}
            onMouseDown={handleDragStart}
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize sidebar"
          >
            <div
              className="h-full w-px opacity-0 transition-opacity hover:opacity-100"
              style={{ background: "var(--color-salient)" }}
            />
          </div>
        )}

        {expanded && (
          <div
            className="flex min-h-0 flex-1 flex-col overflow-hidden"
            style={{ width: width }}
          >
            {sidebarContent}
          </div>
        )}

        {/* Icon rail (always visible) */}
        <div
          className="flex flex-shrink-0 flex-col items-center gap-1 border-l pt-2"
          style={{
            width: SIDEBAR_COLLAPSED_WIDTH,
            borderColor: expanded ? "var(--color-border)" : "transparent",
          }}
        >
          <IconButton
            icon={<Layers className="h-4 w-4" />}
            active={expanded && tab === "episodes"}
            label="Episodes"
            onClick={() => handleTabClick("episodes")}
          />
          <IconButton
            icon={<Search className="h-4 w-4" />}
            active={expanded && tab === "search"}
            label="Search memories"
            onClick={() => handleTabClick("search")}
          />
        </div>
      </aside>
    </>
  );
}

function IconButton({
  icon,
  active,
  label,
  onClick,
}: {
  icon: React.ReactNode;
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="flex h-7 w-7 items-center justify-center rounded-lg transition-all"
      style={{
        color: active
          ? "var(--color-salient)"
          : "var(--color-text-tertiary)",
        background: active ? "var(--color-surface-raised)" : "transparent",
        boxShadow: active ? "var(--shadow-sm)" : "none",
      }}
      title={label}
      aria-label={label}
      aria-pressed={active}
    >
      {icon}
    </button>
  );
}

function MobileTabButton({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className="flex items-center gap-2 rounded-full border px-3 py-2"
      style={{
        borderColor: active ? "var(--color-salient)" : "var(--color-border)",
        background: active ? "var(--color-salient-glow)" : "var(--color-surface-raised)",
        color: active ? "var(--color-salient)" : "var(--color-text-secondary)",
        fontSize: "var(--font-size-xs)",
      }}
      aria-pressed={active}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}
