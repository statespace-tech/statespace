import { version } from './version.js';

const DEFAULT_BASE_URL = "https://api.statespace.com";

export interface SearchResult {
  url: string;
  site: string;
  title: string;
  snippet: string;
}

export interface SearchResponse {
  results: SearchResult[];
  total: number;
}

export interface SearchOptions {
  limit?: number;
  /** @internal */
  baseUrl?: string;
}

export async function search(query: string, options: SearchOptions = {}): Promise<SearchResponse> {
  const { limit = 10, baseUrl = DEFAULT_BASE_URL } = options;

  const url = new URL(`${baseUrl}/search`);
  url.searchParams.set("q", query);
  url.searchParams.set("limit", String(limit));

  const response = await fetch(url.toString(), {
    headers: { "User-Agent": `statespace-sdk/${version}` },
  });

  if (!response.ok) {
    const body = await response.json().catch(() => null) as { error?: string } | null;
    throw new Error(body?.error ?? `HTTP ${response.status}`);
  }

  return response.json() as Promise<SearchResponse>;
}
