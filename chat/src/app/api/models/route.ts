/**
 * Proxies OpenRouter's model list with server-side caching.
 * Avoids CORS issues and reduces client-side requests.
 * Revalidates every hour via Next.js ISR.
 */

export const revalidate = 3600; // 1 hour ISR

interface OpenRouterModel {
  id: string;
  name: string;
  context_length: number;
  pricing: {
    prompt: string;
    completion: string;
  };
  architecture?: {
    input_modalities?: string[];
  };
}

export interface TransformedModel {
  id: string;
  name: string;
  provider: string;
  contextLength: number;
  promptPrice: number;
  completionPrice: number;
}

function transformModel(raw: OpenRouterModel): TransformedModel {
  const provider = raw.id.split("/")[0] ?? "unknown";
  return {
    id: raw.id,
    name: raw.name,
    provider,
    contextLength: raw.context_length,
    promptPrice: parseFloat(raw.pricing?.prompt ?? "0"),
    completionPrice: parseFloat(raw.pricing?.completion ?? "0"),
  };
}

export async function GET() {
  try {
    const res = await fetch("https://openrouter.ai/api/v1/models", {
      next: { revalidate: 3600 },
    });

    if (!res.ok) {
      return Response.json(
        { error: "OpenRouter API returned " + res.status },
        { status: 502 }
      );
    }

    const json = await res.json();
    const models: TransformedModel[] = (json.data as OpenRouterModel[])
      .filter((m) => m.pricing?.prompt !== undefined)
      .map(transformModel)
      .sort((a, b) => a.name.localeCompare(b.name));

    return Response.json({ models });
  } catch {
    return Response.json(
      { error: "Failed to fetch models from OpenRouter" },
      { status: 502 }
    );
  }
}
