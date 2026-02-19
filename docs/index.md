---
icon: lucide/home
---


# Get started with Statespace

Build interactive web apps for AI agents in Markdown.

Statespace is a declarative framework turns your Markdown files into shareable web apps that any AI agent can interact with.
Build and share RAG pipelines, text-to-SQL interfaces, knowledge bases, chatbots, agent skills, and more.

## Example

The following app is running on [https://demo.statespace.app](https://demo.app.statespace.com):

````yaml title="README.md"
---
tools:
  - [expr, { }]
---

```component
echo "Random number: $RANDOM"
```

# Instructions
- The component loads a random number when the page loads
- Use the `expr` tool to multiply it
````

Pass the app URL to your coding agent to check it out:

=== ":simple-claude: &nbsp; Claude Code"

    ```console
    $ claude "Multiply the random number in http://demo.app.statespace.com by 256"
    ```

=== ":simple-githubcopilot: &nbsp; GitHub Copilot"

    ```console
    $ copilot -[] "Multiply the random number in http://demo.app.statespace.com by 256"
    ```

=== ":simple-cursor: &nbsp; Cursor"

    ```console
    $ agent "Multiply the random number in http://demo.app.statespace.com by 256"
    ```


Alternatively, try it locally: 

1. Save the example above as `myapp/README.md`
2. Run `statespace serve myapp/`
3. Point your agent to [`http://127.0.0.1:8000`](http://127.0.0.1:8000)

> **Note**: Statespace apps work with any agent that that can `curl` URLs.

## Concepts

<div class="grid cards concept-cards" markdown style="grid-template-columns: repeat(1, 1fr);">

-   :lucide-file:{ .md .middle } &nbsp; [__Pages__](pages/develop/pages.md#overview)

    ---

    Markdown files served over HTTP. Write instructions, documentation, and context to guide agents.

-   :lucide-wrench:{ .md .middle } &nbsp; [__Tools__](pages/develop/tools.md#overview)

    ---

    CLI commands that agents can call via HTTP. Query databases, call APIs, run scripts, or execute any shell command.

-   :lucide-sparkles:{ .md .middle } &nbsp; [__Components__](pages/develop/components.md#overview)

    ---

    Shell commands embedded in pages that run when the page loads. Render live data like query results or system status.

</div>

## Features

**Lightweight** - Just Markdown files and a single Rust binary. No dependencies.

**Universal** - Works immediately with [any agent](pages/connect/agents.md) that can make HTTP requests.

**Portable** - [Deploy to the cloud](pages/deploy/cloud.md) for a public URL, or [run locally](pages/deploy/self_hosting.md) with `statespace serve`.

**Secure** - Restrict access to your private apps with [token-based authentication](pages/deploy/security.md).

**Debuggable** - [Tunnel via SSH](pages/connect/ssh.md) to debug and patch deployed applications.

## Use cases

<div class="grid cards" markdown style="grid-template-columns: repeat(3, 1fr);">

-   :lucide-file-stack:{ .md .middle .jade } &nbsp; **RAG**

    ---

    Search documents with `grep`, `cat`, or your APIs.

-   :lucide-database:{ .md .middle .jade } &nbsp; **Text-to-SQL**

    ---

    Query databases with read-only access.

-   :lucide-bot-message-square:{ .md .middle .jade } &nbsp; **Chatbots**

    ---

    Build multi-turn conversational flows.

-   :lucide-library:{ .md .middle .jade } &nbsp; **Knowledge bases**

    ---

    Organize records for structured queries.

-   :lucide-sprout:{ .md .middle .jade } &nbsp; **Agent skills**

    ---

    Package tools into reusable skills.

-   :lucide-workflow:{ .md .middle .jade } &nbsp; **Workflows**

    ---

    Chain actions into multi-step flows.

</div>
