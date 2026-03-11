/**
 * assistant-ui LocalRuntime adapter for AM backend.
 *
 * Bridges assistant-ui's ChatModelAdapter interface to AM's /api/chat SSE endpoint.
 * Stores DAE recall metadata per-message for the memory context panel (ALP-1138).
 *
 * Error classification (ALP-1157):
 *   - rate-limit: OpenRouter 429 during streaming
 *   - server-error: AM backend crash or 5xx
 *   - network-error: fetch failure or stream read failure
 *   - stream-error: SSE error event from backend
 */

import { useMemo } from "react";
import type { ChatModelAdapter } from "@assistant-ui/react";
import { useLocalRuntime } from "@assistant-ui/react";
import { chatStream } from "./am-client";
import { parseSSEStream } from "./sse-parser";
import type { ChatMessage, ContextMetadata } from "./types";

// --- Streaming error types ---

export type StreamingErrorType =
  | "rate-limit"
  | "server-error"
  | "network-error"
  | "stream-error";

export interface StreamingErrorInfo {
  type: StreamingErrorType;
  message: string;
  suggestion: string;
}

/**
 * Classify an error into a user-facing streaming error.
 * The returned object is serialized into the assistant-ui message
 * status.error field for the error display component to read.
 */
function classifyError(err: unknown): StreamingErrorInfo {
  if (err instanceof TypeError && String(err.message).includes("fetch")) {
    return {
      type: "network-error",
      message: "Network connection interrupted",
      suggestion: "Check your connection and try again.",
    };
  }

  if (err instanceof Error) {
    const msg = err.message;

    // Rate limit (429) from OpenRouter via AM backend
    if (msg.includes("429") || msg.toLowerCase().includes("rate limit")) {
      return {
        type: "rate-limit",
        message: "The AI provider hit a rate limit",
        suggestion: "Try again in a few seconds.",
      };
    }

    // Server errors (5xx)
    if (/\b5\d{2}\b/.test(msg) || msg.includes("ECONNREFUSED")) {
      return {
        type: "server-error",
        message: "Lost connection to memory server",
        suggestion: "The AM server may have restarted. Try again.",
      };
    }

    // Network errors (various fetch/stream failure patterns)
    if (
      msg.includes("network") ||
      msg.includes("aborted") ||
      msg.includes("ECONNRESET") ||
      msg.includes("Failed to fetch") ||
      msg.includes("NetworkError")
    ) {
      return {
        type: "network-error",
        message: "Network connection interrupted",
        suggestion: "Check your connection and try again.",
      };
    }

    // SSE error event from backend
    if (msg.startsWith("AM error")) {
      return {
        type: "stream-error",
        message: msg.replace(/^AM error \[.*?\]: /, ""),
        suggestion: "Try again. If this persists, the model may be unavailable.",
      };
    }
  }

  // Fallback
  return {
    type: "stream-error",
    message: "Something went wrong during streaming",
    suggestion: "Try again.",
  };
}

/**
 * Per-message context metadata, keyed by assistant message ID.
 * Stored in a module-level Map so the memory panel can access it.
 */
const contextStore = new Map<string, ContextMetadata>();
const queryStore = new Map<string, string>();

export function getContextForMessage(
  messageId: string
): ContextMetadata | undefined {
  return contextStore.get(messageId);
}

export function getQueryForMessage(
  messageId: string
): string {
  return queryStore.get(messageId) ?? "";
}

export function clearContextStore(): void {
  contextStore.clear();
  queryStore.clear();
}

interface AMAdapterOptions {
  getApiKey: () => string | undefined;
  getModel: () => string | undefined;
  getMode: () => "explorer" | "assistant";
}

/**
 * Create a ChatModelAdapter that talks to the AM backend.
 * Errors are classified and attached as structured JSON to the
 * message status so the UI can render appropriate error states.
 */
export function createAMAdapter(options: AMAdapterOptions): ChatModelAdapter {
  return {
    async *run({ messages, abortSignal, unstable_assistantMessageId }) {
      const apiKey = options.getApiKey();
      const model = options.getModel();
      const mode = options.getMode();

      // Convert assistant-ui messages to AM ChatMessage format
      const conversation: ChatMessage[] = [];
      let lastUserText = "";

      for (const msg of messages) {
        const textParts = msg.content.filter(
          (p): p is { type: "text"; text: string } => p.type === "text"
        );
        const text = textParts.map((p) => p.text).join("");

        if (msg.role === "user") {
          conversation.push({ role: "user", content: text });
          lastUserText = text;
        } else if (msg.role === "assistant") {
          conversation.push({ role: "assistant", content: text });
        }
      }

      if (!lastUserText) return;

      let response: Response;
      try {
        response = await chatStream(
          {
            message: lastUserText,
            conversation,
            model: model ?? undefined,
            mode,
          },
          apiKey,
          abortSignal
        );
      } catch (err) {
        // Pre-stream failure (connection refused, 429 before streaming, etc.)
        throw classifyError(err);
      }

      if (!response.body) {
        throw classifyError(new Error("No response body from AM backend"));
      }

      let accumulated = "";

      try {
        for await (const event of parseSSEStream(response.body)) {
          if (abortSignal.aborted) break;

          switch (event.type) {
            case "context": {
              // Store context metadata and user query for the memory panel
              if (unstable_assistantMessageId) {
                contextStore.set(unstable_assistantMessageId, event.json);
                queryStore.set(unstable_assistantMessageId, lastUserText);
              }
              break;
            }
            case "data": {
              accumulated += event.text;
              yield {
                content: [{ type: "text" as const, text: accumulated }],
              };
              break;
            }
            case "error": {
              throw new Error(
                `AM error [${event.json.code}]: ${event.json.message}`
              );
            }
            case "done": {
              return;
            }
          }
        }
      } catch (err) {
        // Mid-stream failure: classify and re-throw.
        // Partial content is already yielded and preserved by assistant-ui.
        if (abortSignal.aborted) return;
        throw classifyError(err);
      }
    },
  };
}

/**
 * React hook that creates a LocalRuntime wired to the AM backend.
 *
 * @param options - Configuration callbacks for API key, model, and mode
 * @returns AssistantRuntime instance for use with AssistantRuntimeProvider
 */
export function useAMRuntime(options: AMAdapterOptions) {
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const adapter = useMemo(() => createAMAdapter(options), [options]);
  return useLocalRuntime(adapter);
}
