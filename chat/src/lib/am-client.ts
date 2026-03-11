/**
 * AM HTTP API client.
 * Placeholder - full implementation in ALP-1136.
 */

export const AM_API_URL =
  process.env.NEXT_PUBLIC_AM_API_URL ?? "http://localhost:3001";

export interface AmHealthResponse {
  ok: boolean;
}

export async function checkHealth(): Promise<AmHealthResponse> {
  const res = await fetch(`${AM_API_URL}/api/health`);
  if (!res.ok) throw new Error(`AM health check failed: ${res.status}`);
  return res.json() as Promise<AmHealthResponse>;
}
