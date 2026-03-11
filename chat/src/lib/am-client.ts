/**
 * Typed client for AM HTTP API endpoints.
 * Uses NEXT_PUBLIC_AM_API_URL env var for the backend URL.
 */

import type {
  BufferRequest,
  ChatRequest,
  Episode,
  FeedbackRequest,
  HealthResponse,
  IngestRequest,
  QueryIndexRequest,
  QueryIndexResponse,
  QueryRequest,
  QueryResponse,
  RetrieveRequest,
  SalientRequest,
  StatsResponse,
} from "./types";

export const AM_API_URL =
  process.env.NEXT_PUBLIC_AM_API_URL ?? "http://localhost:3001";

class AMClientError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string
  ) {
    super(message);
    this.name = "AMClientError";
  }
}

async function post<T>(
  path: string,
  body: unknown,
  apiKey?: string
): Promise<T> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (apiKey) {
    headers["Authorization"] = `Bearer ${apiKey}`;
  }

  const res = await fetch(`${AM_API_URL}${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });

  if (!res.ok) {
    const text = await res.text();
    let code = "UNKNOWN";
    let message = text;
    try {
      const parsed = JSON.parse(text);
      code = parsed.code ?? code;
      message = parsed.message ?? message;
    } catch {
      // Not JSON, use raw text
    }
    throw new AMClientError(res.status, code, message);
  }

  return res.json() as Promise<T>;
}

async function get<T>(path: string, apiKey?: string): Promise<T> {
  const headers: Record<string, string> = {};
  if (apiKey) {
    headers["Authorization"] = `Bearer ${apiKey}`;
  }

  const res = await fetch(`${AM_API_URL}${path}`, { headers });
  if (!res.ok) {
    throw new AMClientError(res.status, "FETCH_ERROR", `GET ${path} failed: ${res.status}`);
  }
  return res.json() as Promise<T>;
}

/**
 * Stream a chat request to the AM backend.
 * Returns the raw Response for SSE parsing.
 */
export async function chatStream(
  req: ChatRequest,
  apiKey?: string,
  signal?: AbortSignal
): Promise<Response> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (apiKey) {
    headers["Authorization"] = `Bearer ${apiKey}`;
  }

  const res = await fetch(`${AM_API_URL}/api/chat`, {
    method: "POST",
    headers,
    body: JSON.stringify(req),
    signal,
  });

  if (!res.ok) {
    const text = await res.text();
    throw new AMClientError(res.status, "CHAT_ERROR", text);
  }

  return res;
}

// --- Memory Tool Endpoints ---

export function amQuery(
  req: QueryRequest,
  apiKey?: string
): Promise<QueryResponse> {
  return post("/api/am/query", req, apiKey);
}

export function amQueryIndex(
  req: QueryIndexRequest,
  apiKey?: string
): Promise<QueryIndexResponse> {
  return post("/api/am/query-index", req, apiKey);
}

export function amRetrieve(
  req: RetrieveRequest,
  apiKey?: string
): Promise<unknown> {
  return post("/api/am/retrieve", req, apiKey);
}

export function amBuffer(
  req: BufferRequest,
  apiKey?: string
): Promise<unknown> {
  return post("/api/am/buffer", req, apiKey);
}

export function amIngest(
  req: IngestRequest,
  apiKey?: string
): Promise<unknown> {
  return post("/api/am/ingest", req, apiKey);
}

export function amActivate(
  text: string,
  apiKey?: string
): Promise<unknown> {
  return post("/api/am/activate", { text }, apiKey);
}

export function amSalient(
  req: SalientRequest,
  apiKey?: string
): Promise<unknown> {
  return post("/api/am/salient", req, apiKey);
}

export function amFeedback(
  req: FeedbackRequest,
  apiKey?: string
): Promise<unknown> {
  return post("/api/am/feedback", req, apiKey);
}

export function amStats(apiKey?: string): Promise<StatsResponse> {
  return get("/api/am/stats", apiKey);
}

export function amEpisodes(apiKey?: string): Promise<Episode[]> {
  return get("/api/am/episodes", apiKey);
}

export function amExport(apiKey?: string): Promise<unknown> {
  return get("/api/am/export", apiKey);
}

export function amHealth(): Promise<HealthResponse> {
  return get("/api/health");
}
