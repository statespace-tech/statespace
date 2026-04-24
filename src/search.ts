import { version } from "./version.js";
import { DEFAULT_BASE_URL, DEFAULT_LIMIT } from "./constants.js";

interface Result {
  url: string;
  site: string;
  title: string;
  snippet: string;
}

export async function runSearch(argv: string[]): Promise<void> {
  const positional: string[] = [];
  let limit = DEFAULT_LIMIT;
  let human = false;
  let baseUrl = DEFAULT_BASE_URL;

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      process.stdout.write(
        "Usage: statespace search <query> [options]\n\n" +
          "Query syntax:\n" +
          "  <query>              Search all pages across all sites\n" +
          "  <site>: <query>      Match site name in title, query in content\n\n" +
          "Options:\n" +
          "  --limit, -l <n>     Max results (default: 10)\n" +
          "  --human             Human-readable output instead of JSON\n" +
          "  --help,  -h         Show this help\n"
      );
      process.exit(0);
    } else if (arg === "--limit" || arg === "-l") {
      limit = parseInt(argv[++i] ?? "10", 10);
    } else if (arg === "--human") {
      human = true;
    } else if (arg === "--url" || arg === "-u") {
      baseUrl = argv[++i] ?? baseUrl;
    } else if (!arg.startsWith("-")) {
      positional.push(arg);
    }
  }

  const query = positional.join(" ").trim();
  if (!query) {
    process.stderr.write("Error: query is required\nUsage: statespace search <query>\n");
    process.exit(1);
  }

  const url = new URL(`${baseUrl}/search`);
  url.searchParams.set("q", query);
  url.searchParams.set("limit", String(limit));

  let data: { results: Result[]; total: number };
  try {
    const res = await fetch(url.toString(), {
      headers: {
        "User-Agent": `statespace-cli/${version}`,
      },
    });
    if (!res.ok) {
      const body = (await res.json().catch(() => null)) as { error?: string } | null;
      const msg = body?.error ?? `HTTP ${res.status}`;
      process.stderr.write(`Error: ${msg}\n`);
      process.exit(1);
    }
    const raw = (await res.json()) as Record<string, unknown>;
    if (!raw || !Array.isArray(raw.results)) {
      process.stderr.write("Error: unexpected response format from server\n");
      process.exit(1);
    }
    data = raw as { results: Result[]; total: number };
  } catch (e) {
    process.stderr.write(`Error: ${(e as Error).message}\n`);
    process.exit(1);
  }

  if (data.results.length === 0) {
    process.stdout.write(human ? "no results\n" : JSON.stringify([]) + "\n");
    return;
  }

  if (human) {
    for (const r of data.results) {
      process.stdout.write(
        `[${r.site}] ${r.title} — ${r.url}${r.snippet ? ` — ${r.snippet}` : ""}\n`
      );
    }
  } else {
    process.stdout.write(JSON.stringify(data.results, null, 2) + "\n");
  }
}
