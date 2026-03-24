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
  conscious: RecallEntry[];
  subconscious: RecallEntry[];
  novel: RecallEntry[];
}

export interface RecallEntry {
  id: string;
  seed: string;
  score: number;
  text: string;
  is_conscious: boolean;
  category: "Conscious" | "Subconscious" | "Novel";
  type: string;
  epoch: number;
  token_estimate: number;
}

// --- AM REST Endpoints ---

export interface QueryRequest {
  text: string;
  max_tokens?: number;
}

export interface QueryResponse {
  context: string;
  conscious: ContextMetadata["conscious"];
  subconscious: ContextMetadata["subconscious"];
  novel: ContextMetadata["novel"];
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

export interface QueryIndexEntry {
  id: string;
  category: string;
  type: string;
  score: number;
  epoch: number;
  summary: string;
  token_estimate: number;
}

export interface QueryIndexResponse {
  entries: QueryIndexEntry[];
  total_candidates: number;
  total_tokens_if_fetched: number;
}

export interface RetrieveEntry {
  id: string;
  category: string;
  type: string;
  episode: string;
  tokens: number;
  text: string;
}

export interface RetrieveResponse {
  entries: RetrieveEntry[];
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
  is_conscious?: boolean;
}

export interface EpisodeNeighborhood {
  id: string;
  type: string;
  epoch: number;
  tokens: number;
  text: string;
  episode: string;
  is_conscious: boolean;
  superseded_by: string | null;
}

export interface HealthResponse {
  ok: boolean;
}
