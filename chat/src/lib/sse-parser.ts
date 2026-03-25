/**
 * Lightweight SSE line parser for AM backend streams.
 *
 * Handles the AM SSE protocol:
 *   event: context  - JSON metadata (first event)
 *   data: <chunk>   - Content text tokens
 *   data: [DONE]    - End sentinel
 *   event: error    - JSON {code, message}
 *   : keepalive     - Comment frames (ignored)
 *
 * Per the SSE spec, multiple `data:` lines within one event block
 * are concatenated with `\n`. This preserves newlines in streamed content.
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
        let eventType = "";
        const dataLines: string[] = [];

        for (const line of lines) {
          // Comment line (keepalive) - ignore
          if (line.startsWith(":")) {
            continue;
          }

          // Event type line
          if (line.startsWith("event: ")) {
            eventType = line.slice(7).trim();
            continue;
          }

          // Data line (with content after "data: ")
          if (line.startsWith("data: ")) {
            dataLines.push(line.slice(6));
            continue;
          }

          // Data line with no content ("data:" or "data: " trimmed)
          if (line === "data:" || line === "data:") {
            dataLines.push("");
            continue;
          }
        }

        if (dataLines.length === 0) continue;

        // Per SSE spec: join multiple data lines with \n
        const data = dataLines.join("\n");

        if (eventType === "context") {
          try {
            const json = JSON.parse(data) as ContextMetadata;
            yield { type: "context", json };
          } catch {
            // Malformed context metadata - skip
          }
          continue;
        }

        if (eventType === "error") {
          try {
            const json = JSON.parse(data) as SSEError;
            yield { type: "error", json };
          } catch {
            yield {
              type: "error",
              json: { code: "PARSE_ERROR", message: data },
            };
          }
          continue;
        }

        // Regular data or done sentinel
        if (data === "[DONE]") {
          yield { type: "done" };
          return;
        }

        yield { type: "data", text: data };
      }
    }
  } finally {
    reader.releaseLock();
  }
}
