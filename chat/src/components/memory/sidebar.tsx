"use client";

import {
  useState,
  useEffect,
  useCallback,
  useRef,
} from "react";
import { Layers, Search } from "lucide-react";
import { StatsHeader } from "./stats-header";
import { EpisodeList } from "./episode-list";
import type { Episode } from "@/lib/types";

type SidebarTab = "episodes" | "search";

const STORAGE_KEY = "am_sidebar_tab";
const SIDEBAR_COLLAPSED_WIDTH = 32;
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
  onSelectEpisode?: (episode: Episode) => void;
  onUploadClick?: () => void;
  /** Controlled search content from parent. Null when no search panel exists yet. */
  searchContent?: React.ReactNode;
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
export function Sidebar({ onSelectEpisode, onUploadClick, searchContent }: SidebarProps) {
  const [expanded, setExpanded] = useState(false);
  const [tab, setTab] = useState<SidebarTab>(loadTab);
  const [width, setWidth] = useState(SIDEBAR_DEFAULT_WIDTH);
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

  return (
    <aside
      ref={sidebarRef}
      className="relative hidden flex-shrink-0 border-l lg:flex"
      style={{
        width: sidebarWidth,
        borderColor: "var(--color-border)",
        background: "var(--color-surface)",
        transition: isDragging.current ? "none" : "width var(--transition-normal)",
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

      {/* Expanded panel */}
      {expanded && (
        <div
          className="flex flex-1 flex-col overflow-hidden"
          style={{ width: width }}
        >
          {/* Stats header */}
          <div
            className="flex-shrink-0 border-b"
            style={{ borderColor: "var(--color-border)" }}
          >
            <StatsHeader />
          </div>

          {/* Tab content */}
          <div className="flex-1 overflow-hidden pt-2">
            {tab === "episodes" && (
              <EpisodeList
                onSelectEpisode={onSelectEpisode}
                onUploadClick={onUploadClick}
              />
            )}
            {tab === "search" && (
              searchContent ?? (
                <div className="px-3 py-4">
                  <p
                    className="text-center text-xs"
                    style={{ color: "var(--color-text-secondary)" }}
                  >
                    Memory search
                  </p>
                </div>
              )
            )}
          </div>
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
      className="flex h-7 w-7 items-center justify-center rounded transition-colors"
      style={{
        color: active
          ? "var(--color-salient)"
          : "var(--color-text-secondary)",
        background: active ? "var(--color-surface-raised)" : "transparent",
      }}
      title={label}
      aria-label={label}
      aria-pressed={active}
    >
      {icon}
    </button>
  );
}
