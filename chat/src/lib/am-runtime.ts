/**
 * assistant-ui LocalRuntime adapter for AM backend.
 *
 * Bridges assistant-ui's ChatModelAdapter interface to AM's /api/chat SSE endpoint.
 * Stores DAE recall metadata per-message for the memory context panel (ALP-1138).
 */

import type { ChatModelAdapter } from "@assistant-ui/react";
import { useLocalRuntime } from "@assistant-ui/react";
import { chatStream } from "./am-client";
import { parseSSEStream } from "./sse-parser";
import type { ChatMessage, ContextMetadata } from "./types";

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

      const response = await chatStream(
        {
          message: lastUserText,
          conversation,
          model: model ?? undefined,
          mode,
        },
        apiKey,
        abortSignal
      );

      if (!response.body) {
        throw new Error("No response body from AM backend");
      }

      let accumulated = "";

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
  const adapter = createAMAdapter(options);
  return useLocalRuntime(adapter);
}
