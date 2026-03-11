/**
 * assistant-ui LocalRuntime adapter for AM backend.
 * Placeholder with mock adapter - full implementation in ALP-1136.
 */

import type { ChatModelAdapter } from "@assistant-ui/react";

export const mockAdapter: ChatModelAdapter = {
  async *run({ messages, abortSignal }) {
    // Simulate streaming delay for the mock
    const userMessage = messages[messages.length - 1];
    const userText =
      userMessage?.content
        ?.filter(
          (p): p is { type: "text"; text: string } => p.type === "text"
        )
        .map((p) => p.text)
        .join("") ?? "";

    const response = `This is a mock response. You said: "${userText}"\n\nThe AM backend is not connected yet. This placeholder will be replaced by the real runtime adapter in ALP-1136.`;

    // Simulate token-by-token streaming
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
