import { version } from "./version.js";
import { DEFAULT_BASE_URL, DEFAULT_LIMIT } from "./constants.js";

export interface SearchResult {
  url: string;
  site: string;
  title: string;
  snippet: string;
}

export interface SearchOptions {
  limit?: number;
  /** @internal */
  baseUrl?: string;
}

export async function search(query: string, options: SearchOptions = {}): Promise<SearchResult[]> {
  const { limit = DEFAULT_LIMIT, baseUrl = DEFAULT_BASE_URL } = options;

  const url = new URL(`${baseUrl}/search`);
  url.searchParams.set("q", query);
  url.searchParams.set("limit", String(limit));

  const response = await fetch(url.toString(), {
    headers: { "User-Agent": `statespace-sdk/${version}` },
  });

  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? `HTTP ${response.status}`);
  }

  const data = (await response.json()) as { results: SearchResult[]; total: number };
  return data.results;
}
