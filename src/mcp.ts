import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { SSEServerTransport } from "@modelcontextprotocol/sdk/server/sse.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import { createServer } from "http";
import { getClientId } from './client_id.js';

function buildServer(baseUrl: string): Server {
  const server = new Server(
    { name: "statespace", version: "0.1.0" },
    { capabilities: { tools: {} } }
  );

  server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: [
      {
        name: "search",
        description:
          "Search documentation indexed from llms.txt sites. " +
          "Use plain queries to search all pages, or 'site: query' syntax to match a site by name and search within it (e.g. 'supabase: edge functions').",
        inputSchema: {
          type: "object" as const,
          properties: {
            q: {
              type: "string",
              description: "Search query. Use 'site: query' syntax to target a specific site.",
            },
            limit: {
              type: "integer",
              description: "Max results to return (default: 10)",
              default: 10,
            },
          },
          required: ["q"],
        },
      },
    ],
  }));

  server.setRequestHandler(CallToolRequestSchema, async (request) => {
    const { name, arguments: args } = request.params;

    if (name !== "search") {
      return {
        content: [{ type: "text" as const, text: `Unknown tool: ${name}` }],
        isError: true,
      };
    }

    const q = args?.["q"] as string | undefined;
    if (!q) {
      return {
        content: [{ type: "text" as const, text: "Error: q is required" }],
        isError: true,
      };
    }

    const limit = (args?.["limit"] as number | undefined) ?? 10;

    const url = new URL(`${baseUrl}/search`);
    url.searchParams.set("q", q);
    url.searchParams.set("limit", String(limit));

    try {
      const response = await fetch(url.toString(), {
        headers: {
          'User-Agent': 'statespace-mcp/0.1.0',
          'X-Client-Id': getClientId(),
        },
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const results = await response.json();
      return {
        content: [{ type: "text" as const, text: JSON.stringify(results, null, 2) }],
      };
    } catch (e) {
      return {
        content: [{ type: "text" as const, text: `Error: ${(e as Error).message}` }],
        isError: true,
      };
    }
  });

  return server;
}

export async function runMcp(argv: string[]): Promise<void> {
  let baseUrl = "http://localhost:3000";
  let transport = "stdio";
  let port = 4000;

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      process.stdout.write(
        "Usage: statespace mcp [options]\n\n" +
        "Options:\n" +
        "  --url <url>            Backend API base URL (default: http://localhost:3000)\n" +
        "  --transport <mode>     Transport mode: stdio or sse (default: stdio)\n" +
        "  --port <n>             Port for SSE transport (default: 4000)\n" +
        "  --help, -h             Show this help\n"
      );
      process.exit(0);
    } else if (arg === "--url" && argv[i + 1]) {
      baseUrl = argv[++i];
    } else if (arg === "--transport" && argv[i + 1]) {
      transport = argv[++i];
    } else if (arg === "--port" && argv[i + 1]) {
      port = parseInt(argv[++i], 10);
    }
  }

  if (transport === "sse") {
    const sessions = new Map<string, SSEServerTransport>();

    const httpServer = createServer(async (req, res) => {
      if (req.method === "GET" && req.url === "/sse") {
        const t = new SSEServerTransport("/message", res);
        sessions.set(t.sessionId, t);
        res.on("close", () => sessions.delete(t.sessionId));
        await buildServer(baseUrl).connect(t);
      } else if (req.method === "POST" && req.url?.startsWith("/message")) {
        const sessionId =
          new URL(req.url, "http://x").searchParams.get("sessionId") ?? "";
        const t = sessions.get(sessionId);
        if (t) {
          await t.handlePostMessage(req, res);
        } else {
          res.writeHead(404).end("session not found");
        }
      } else {
        res.writeHead(404).end();
      }
    });

    httpServer.listen(port, () => {
      process.stderr.write(`MCP SSE server listening on :${port}\n`);
    });
  } else {
    await buildServer(baseUrl).connect(new StdioServerTransport());
  }
}
