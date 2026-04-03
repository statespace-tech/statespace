---
icon: simple/modelcontextprotocol
---

# Connect via MCP

Statespace apps can be wired up as MCP (Model Context Protocol) servers, so agents can use them directly from their environment without making raw HTTP requests.

## How it works

The MCP server exposes two tools:

- **`read_page`** — reads any file from the app and returns its raw content. Agents start with `README.md` and follow links from there.
- **`run_command`** — executes a command declared in a page's YAML frontmatter. Agents call `read_page` first to discover what commands are available.

## Setup

Add your deployed app to your MCP config:

```json
"mcpServers": {
  "my-app": {
    "command": "npx",
    "args": ["-y", "statespace-mcp", "https://my-app.statespace.app"]
  }
}
```

For private apps, pass the access token:

```json
"mcpServers": {
  "my-app": {
    "command": "npx",
    "args": ["-y", "statespace-mcp", "https://my-app.statespace.app"],
    "env": {
      "STATESPACE_TOKEN": "<TOKEN>"
    }
  }
}
```

## Local apps

You can also point the MCP server at a locally running app:

```json
"mcpServers": {
  "my-app": {
    "command": "npx",
    "args": ["-y", "statespace-mcp", "http://localhost:8000"]
  }
}
```
