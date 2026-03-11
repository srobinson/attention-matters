/**
 * Shared AM response types.
 * Mirrors the Rust backend request/response shapes.
 */

// --- SSE Event Types ---

export type SSEEvent =
  | { type: "context"; json: ContextMetadata }
  | { type: "data"; text: string }
  | { type: "error"; json: SSEError }
  | { type: "done" };

export interface SSEError {
  code: string;
  message: string;
}

// --- Chat ---

export interface ChatRequest {
  message: string;
  conversation: ChatMessage[];
  model?: string;
  mode?: "explorer" | "assistant";
  max_tokens?: number;
}

export interface ChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

// --- Context Metadata (from event: context) ---

export interface ContextMetadata {
  metrics: {
    conscious: number;
    subconscious: number;
    novel: number;
  } | null;
  recalled_ids: {
    conscious: string[];
    subconscious: string[];
    novel: string[];
  } | null;
  token_estimate: {
    conscious: number;
    subconscious: number;
    novel: number;
    total: number;
  } | null;
  index: RecallEntry[] | null;
}

export interface RecallEntry {
  id: string;
  category: "Conscious" | "Subconscious" | "Novel";
  score: number;
  summary: string;
  token_estimate: number;
  epoch: number;
  type: string;
}

// --- AM REST Endpoints ---

export interface QueryRequest {
  text: string;
  max_tokens?: number;
}

export interface QueryResponse {
  context: string;
  metrics: ContextMetadata["metrics"];
  recalled_ids: ContextMetadata["recalled_ids"];
  token_estimate: ContextMetadata["token_estimate"];
  index: RecallEntry[];
  stats: {
    episodes: number;
    n: number;
    conscious: number;
  };
}

export interface QueryIndexRequest {
  text: string;
}

export interface QueryIndexResponse {
  results: Array<{
    id: string;
    score: number;
    summary: string;
    epoch: number;
    type: string;
  }>;
}

export interface RetrieveRequest {
  ids: string[];
}

export interface BufferRequest {
  user: string;
  assistant: string;
}

export interface IngestRequest {
  text: string;
  name?: string;
}

export interface IngestResponse {
  episode_name: string;
  neighborhoods: number;
  occurrences: number;
}

export interface ImportRequest {
  state: unknown;
}

export interface ImportResponse {
  episodes: number;
  neighborhoods: number;
  occurrences: number;
}

export interface SalientRequest {
  text: string;
  supersedes?: string[];
}

export interface FeedbackRequest {
  query: string;
  neighborhood_ids: string[];
  signal: "boost" | "demote";
}

export interface StatsResponse {
  episodes: number;
  n: number;
  conscious: number;
  db_size_bytes?: number;
  activation?: {
    mean: number;
    max: number;
    zero_count: number;
  };
}

export interface Episode {
  id: string;
  name: string;
  created: string;
  neighborhood_count: number;
  total_occurrences: number;
}

export interface HealthResponse {
  ok: boolean;
}
