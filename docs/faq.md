---
icon: lucide/circle-help
---

# FAQ

## How is Statespace different from MCPs or Agent Skills?

|                    | Statespace     | MCPs               | Agent Skills     |
|--------------------|----------------|--------------------|------------------|
| **Implementation** | Markdown       | Code               | Markdown         |
| **Content**        | Dynamic        | Static             | Static           |
| **Tools**          | Server-side    | Server-side        | N/A              |
| **Discovery**      | Progressive    | Upfront [^1]       | Progressive      |
| **Connection**     | Stateless HTTP | Stateful stdio/SSE | Local filesystem |

## Is Statespace different from agent frameworks?

They're complementary. Agent frameworks help you build agents; Statespace lets you extend them, and share tools and context across them.

## Can I use Statespace with any AI agent?

Yes! Most coding agents (e.g. Claude Code, Cursor, Codex) work out of the box with Statespace. Custom agents just need a `curl` tool to interact with apps. See the [agents](pages/connect/agents.md) page for details.

[^1]: Loading all tool definitions upfront slows down agents and increases costs. See [Anthropic's blog post](https://www.anthropic.com/engineering/code-execution-with-mcp).