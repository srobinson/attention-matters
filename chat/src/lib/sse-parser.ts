/**
 * Lightweight SSE line parser for AM backend streams.
 *
 * Handles the AM SSE protocol:
 *   event: context  - JSON metadata (first event)
 *   data: <chunk>   - Content text tokens
 *   data: [DONE]    - End sentinel
 *   event: error    - JSON {code, message}
 *   : keepalive     - Comment frames (ignored)
 */

import type { SSEEvent, ContextMetadata, SSEError } from "./types";

/**
 * Parse an SSE stream from a ReadableStream<Uint8Array>.
 * Yields typed SSEEvent objects.
 */
export async function* parseSSEStream(
  stream: ReadableStream<Uint8Array>
): AsyncGenerator<SSEEvent> {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let currentEvent = "";

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });

      // SSE events are separated by double newlines
      const parts = buffer.split("\n\n");
      // Keep the last incomplete part in the buffer
      buffer = parts.pop() ?? "";

      for (const part of parts) {
        const lines = part.split("\n");
        // Reset event type at each block boundary per SSE spec
        currentEvent = "";

        for (const line of lines) {
          // Comment line (keepalive) - ignore
          if (line.startsWith(":")) {
            continue;
          }

          // Event type line
          if (line.startsWith("event: ")) {
            currentEvent = line.slice(7).trim();
            continue;
          }

          // Data line
          if (line.startsWith("data: ")) {
            const data = line.slice(6);

            if (currentEvent === "context") {
              try {
                const json = JSON.parse(data) as ContextMetadata;
                yield { type: "context", json };
              } catch {
                // Malformed context metadata - skip
              }
              currentEvent = "";
              continue;
            }

            if (currentEvent === "error") {
              try {
                const json = JSON.parse(data) as SSEError;
                yield { type: "error", json };
              } catch {
                yield {
                  type: "error",
                  json: { code: "PARSE_ERROR", message: data },
                };
              }
              currentEvent = "";
              continue;
            }

            // Regular data or done sentinel
            if (data === "[DONE]") {
              yield { type: "done" };
              return;
            }

            yield { type: "data", text: data };
            currentEvent = "";
          }
        }
      }
    }
  } finally {
    reader.releaseLock();
  }
}
