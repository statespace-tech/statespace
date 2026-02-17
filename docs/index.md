---
icon: lucide/home
---


# Get started with Statespace

Build interactive web apps for AI agents in Markdown.

Statespace turns your Markdown files into shareable web apps that any AI agent can interact with.
With just a few Markdown files, build RAG pipelines, text-to-SQL interfaces, knowledge bases, chatbots, agent skills, and more.


## Example

The following app is running on [https://demo.app.statespace.com](http://demo.app.statespace.com):

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
    $ copilot "Multiply the random number in http://demo.app.statespace.com by 256"
    ```

=== ":simple-cursor: &nbsp; Cursor"

    ```console
    $ agent "Multiply the random number in http://demo.app.statespace.com by 256"
    ```


Alternatively, [run the app locally](pages/deploy/self_hosting.md#quick-start){ data-preview } and point your agent to [http://127.0.0.1](http://127.0.0.1).


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

-   :lucide-file-stack:{ .md .middle } &nbsp; **RAG**

    ---

    Search documents with `grep`, `cat`, or vector APIs.

-   :lucide-database:{ .md .middle } &nbsp; **Text-to-SQL**

    ---

    Give agents read-only database access.

-   :lucide-bot-message-square:{ .md .middle } &nbsp; **Chatbots**

    ---

    Build conversational flows across pages.

-   :lucide-library:{ .md .middle } &nbsp; **Knowledge bases**

    ---

    Structured records agents can query.

-   :lucide-sprout:{ .md .middle } &nbsp; **Agent skills**

    ---

    Package tools into reusable capabilities.

-   :lucide-workflow:{ .md .middle } &nbsp; **Workflows**

    ---

    Orchestrate multi-step processes.

</div>