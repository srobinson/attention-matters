"use client";

import { AlertTriangle, RefreshCw, Wifi, Server, Clock } from "lucide-react";
import { ActionBarPrimitive } from "@assistant-ui/react";
import type { StreamingErrorInfo, StreamingErrorType } from "@/lib/am-runtime";

interface StreamingErrorProps {
  error: unknown;
}

const ERROR_ICONS: Record<StreamingErrorType, typeof AlertTriangle> = {
  "rate-limit": Clock,
  "server-error": Server,
  "network-error": Wifi,
  "stream-error": AlertTriangle,
};

/**
 * Determine whether an error object is a StreamingErrorInfo.
 */
function isStreamingError(err: unknown): err is StreamingErrorInfo {
  return (
    typeof err === "object" &&
    err !== null &&
    "type" in err &&
    "message" in err &&
    "suggestion" in err
  );
}

/**
 * Inline error display for failed streaming responses.
 * Shows error type icon, plain-language message, suggestion,
 * and a retry button that re-sends the last user message.
 *
 * Renders below the partial assistant response content (which
 * is preserved by assistant-ui, not deleted).
 */
export function StreamingError({ error }: StreamingErrorProps) {
  const info: StreamingErrorInfo = isStreamingError(error)
    ? error
    : {
        type: "stream-error",
        message: error instanceof Error ? error.message : "Something went wrong",
        suggestion: "Try again.",
      };

  const Icon = ERROR_ICONS[info.type];

  return (
    <div
      className="mt-2 flex items-start gap-2.5 rounded-lg border px-3 py-2.5"
      style={{
        borderColor: "var(--color-error-muted)",
        background: "var(--color-error-glow)",
      }}
      role="alert"
    >
      <Icon
        className="mt-0.5 h-4 w-4 flex-shrink-0"
        style={{ color: "var(--color-error)" }}
        aria-hidden="true"
      />
      <div className="flex flex-1 flex-col gap-1 min-w-0">
        <p
          className="text-xs font-medium"
          style={{ color: "var(--color-text-primary)" }}
        >
          {info.message}
        </p>
        <p
          className="text-[11px]"
          style={{ color: "var(--color-text-secondary)" }}
        >
          {info.suggestion}
        </p>
      </div>

      <ActionBarPrimitive.Root hideWhenRunning autohide="never">
        <ActionBarPrimitive.Reload asChild>
          <button
            className="flex flex-shrink-0 items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-[11px] font-medium transition-colors hover:opacity-80"
            style={{
              borderColor: "var(--color-border)",
              color: "var(--color-text-primary)",
              background: "var(--color-surface-raised)",
            }}
          >
            <RefreshCw className="h-3 w-3" />
            Retry
          </button>
        </ActionBarPrimitive.Reload>
      </ActionBarPrimitive.Root>
    </div>
  );
}
