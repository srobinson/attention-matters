/**
 * Runtime adapter barrel.
 * Re-exports the AM adapter and provides a mock for development.
 */

import type { ChatModelAdapter } from "@assistant-ui/react";

export { createAMAdapter, useAMRuntime, getContextForMessage, getQueryForMessage } from "./am-runtime";

/**
 * Mock adapter for development without a running AM backend.
 * Simulates streaming token-by-token responses.
 */
export const mockAdapter: ChatModelAdapter = {
  async *run({ messages, abortSignal }) {
    const userMessage = messages[messages.length - 1];
    const userText =
      userMessage?.content
        ?.filter(
          (p): p is { type: "text"; text: string } => p.type === "text"
        )
        .map((p) => p.text)
        .join("") ?? "";

    const response = `This is a mock response. You said: "${userText}"\n\nThe AM backend is not connected yet. Configure your API key in settings to connect.`;

    const words = response.split(" ");
    let accumulated = "";

    for (const word of words) {
      if (abortSignal.aborted) break;
      accumulated += (accumulated ? " " : "") + word;
      yield {
        content: [{ type: "text" as const, text: accumulated }],
      };
      await new Promise((resolve) => setTimeout(resolve, 30));
    }
  },
};
