/**
 * Typed client for AM HTTP API endpoints.
 * Uses NEXT_PUBLIC_AM_API_URL env var for the backend URL.
 */

import type {
  BufferRequest,
  ChatRequest,
  Episode,
  EpisodeNeighborhood,
  FeedbackRequest,
  HealthResponse,
  ImportRequest,
  ImportResponse,
  IngestRequest,
  IngestResponse,
  QueryIndexRequest,
  QueryIndexResponse,
  QueryRequest,
  QueryResponse,
  RetrieveRequest,
  RetrieveResponse,
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
      message = parsed.message || parsed.error || message;
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
    const text = await res.text();
    let code = "FETCH_ERROR";
    let message = text || `GET ${path} failed: ${res.status}`;
    try {
      const parsed = JSON.parse(text);
      code = parsed.code ?? code;
      message = parsed.message || parsed.error || message;
    } catch {
      // Not JSON, use raw text
    }
    throw new AMClientError(res.status, code, message);
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
    let code = "CHAT_ERROR";
    let message = text || `Chat request failed: ${res.status}`;
    try {
      const parsed = JSON.parse(text);
      code = parsed.code ?? code;
      message = parsed.message || parsed.error || message;
    } catch {
      // Not JSON, use raw text
    }
    throw new AMClientError(res.status, code, message);
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
): Promise<RetrieveResponse> {
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
): Promise<IngestResponse> {
  return post("/api/am/ingest", req, apiKey);
}

export function amImport(
  req: ImportRequest,
  apiKey?: string
): Promise<ImportResponse> {
  return post("/api/am/import", req, apiKey);
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

/**
 * Normalize an episode record from the API.
 * Handles both old field names (neighborhoods, occurrences)
 * and new field names (neighborhood_count, total_occurrences).
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function normalizeEpisode(raw: any): Episode {
  return {
    id: raw.id ?? "",
    name: raw.name ?? "",
    created: raw.created ?? "",
    neighborhood_count: raw.neighborhood_count ?? raw.neighborhoods ?? 0,
    total_occurrences: raw.total_occurrences ?? raw.occurrences ?? 0,
    is_conscious: raw.is_conscious ?? false,
  };
}

/**
 * Normalize a neighborhood record from the API.
 * Handles both old format (occurrences count, no type) and new format.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function normalizeNeighborhood(raw: any): EpisodeNeighborhood {
  return {
    id: raw.id ?? "",
    type: raw.type ?? "Memory",
    epoch: raw.epoch ?? 0,
    tokens: raw.tokens ?? raw.occurrences ?? 0,
    text: raw.text ?? "",
    episode: raw.episode ?? "",
    is_conscious: raw.is_conscious ?? false,
    superseded_by: raw.superseded_by ?? null,
  };
}

export async function amEpisodes(apiKey?: string): Promise<Episode[]> {
  const json = await get<unknown>("/api/am/episodes", apiKey);

  let items: unknown[];
  if (Array.isArray(json)) {
    items = json;
  } else if (
    typeof json === "object" &&
    json !== null &&
    "episodes" in json &&
    Array.isArray((json as { episodes?: unknown }).episodes)
  ) {
    items = (json as { episodes: unknown[] }).episodes;
  } else {
    throw new Error("Invalid episodes response");
  }

  return items.map(normalizeEpisode);
}

export async function amEpisodeNeighborhoods(
  episodeId: string,
  apiKey?: string
): Promise<EpisodeNeighborhood[]> {
  const raw = await get<unknown[]>(
    `/api/am/episodes/${encodeURIComponent(episodeId)}/neighborhoods`,
    apiKey,
  );
  return raw.map(normalizeNeighborhood);
}

export function amExport(apiKey?: string): Promise<unknown> {
  return get("/api/am/export", apiKey);
}

export function amHealth(): Promise<HealthResponse> {
  return get("/api/health");
}
