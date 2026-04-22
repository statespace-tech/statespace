import { getClientId } from './client_id.js';

interface Result {
  url: string;
  site: string;
  title: string;
  snippet: string;
}

export async function runSearch(argv: string[]): Promise<void> {
  const positional: string[] = [];
  let limit = 10;
  let baseUrl = "http://localhost:3000";

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
        "  --url,   -u <url>   API base URL (default: http://localhost:3000)\n" +
        "  --help,  -h         Show this help\n"
      );
      process.exit(0);
    } else if (arg === "--limit" || arg === "-l") {
      limit = parseInt(argv[++i] ?? "10", 10);
    } else if (arg === "--url" || arg === "-u") {
      baseUrl = argv[++i] ?? "http://localhost:3000";
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
        'User-Agent': 'statespace-cli/0.1.0',
        'X-Client-Id': getClientId(),
      },
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    data = await res.json() as { results: Result[]; total: number };
  } catch (e) {
    process.stderr.write(`Error: ${(e as Error).message}\n`);
    process.exit(1);
  }

  if (data.results.length === 0) {
    process.stdout.write("no results\n");
    return;
  }

  for (const r of data.results) {
    const label = r.site && r.title
      ? `[${r.site}] ${r.title} — ${r.url}`
      : r.site
      ? `[${r.site}] ${r.url}`
      : r.url;
    process.stdout.write(`${label}\n`);
  }
}
