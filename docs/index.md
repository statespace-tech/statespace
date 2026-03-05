---
icon: lucide/home
---


# Get started with Statespace

Build interactive web apps for AI agents in Markdown.

Statespace is a declarative framework that helps you design AI-friendly web applications that agents can navigate and interact with.
Build and share RAG pipelines, text-to-SQL interfaces, knowledge bases, chatbots, agent skills, and more.

## Example

The following app is running on [https://demo.statespace.app](https://demo.statespace.app):

````yaml title="README.md"
---
tools:
  - [expr]
---

```component
echo "Random number: $RANDOM"
```

# Instructions
- The component loads a random number when the page loads
- Use the `expr` tool to perform calculations with it
````

Pass the app URL to any agent that can make HTTP requests:

=== ":simple-claude: &nbsp; Claude Code"

    ```console
    $ claude "Multiply the random number in https://demo.statespace.app by 256"
    ```

=== ":simple-cursor: &nbsp; Cursor"

    ```console
    $ agent "Multiply the random number in https://demo.statespace.app by 256"
    ```

=== ":simple-githubcopilot: &nbsp; GitHub Copilot"

    ```console
    $ copilot -p "Multiply the random number in https://demo.statespace.app by 256"
    ```

Alternatively, run the app locally:

1. Save the example above as `myapp/README.md`
2. Run `statespace serve myapp/`
3. Point your agent to [`http://127.0.0.1:8000`](http://127.0.0.1:8000)

## Concepts

=== ":lucide-wrench: &nbsp; Tools"
    
    Give agents controlled access to CLI commands over HTTP. [Learn more](pages/develop/tools.md)

    ````yaml title="example.md" hl_lines="1-6"
    ---
    tools:
      - [grep]
      - [curl, -X, GET, { }]
      - [psql, -c, { regex: "^SELECT\\b.*" }]
    ---

    ```component
    echo "Server time: $(date)"
    ```

    # Instructions
    - Use grep to search for logs in ./data
    - Query the database for recent users
    - See [analyze](src/analyze.md) for more workflows
    ````

=== ":lucide-sparkles: &nbsp; Components"

    Render live data inside `component` code blocks. [Learn more](pages/develop/components.md)

    ````yaml title="example.md" hl_lines="8-10"
    ---
    tools:
      - [grep]
      - [curl, -X, GET, { }]
      - [psql, -c, { regex: "^SELECT\\b.*" }]
    ---

    ```component
    echo "Server time: $(date)"
    ```

    # Instructions
    - Use grep to search for logs in ./data
    - Query the database for recent users
    - See [analyze](src/analyze.md) for more workflows
    ````

=== ":lucide-file-text: &nbsp; Instructions"

    Guide agents through your data, workflows, and pages. [Learn more](pages/develop/instructions.md)

    ````yaml title="example.md" hl_lines="12-15"
    ---
    tools:
      - [grep]
      - [curl, -X, GET, { }]
      - [psql, -c, { regex: "^SELECT\\b.*" }]
    ---

    ```component
    echo "Server time: $(date)"
    ```

    # Instructions
    - Use grep to search for logs in ./data
    - Query the database for recent users
    - See [analyze](src/analyze.md) for more workflows
    ````

## Features

**Simple** - It's just Markdown. Easy to learn, easy to use, easy to maintain.

**Lightweight** - [Install](install.md) a single Rust binary. No dependencies.

**Universal** - Works immediately with [any agent](pages/connect/agents.md) that can make HTTPS requests.

**Portable** - [Deploy apps to the cloud](pages/deploy/cloud.md) for a public URL, or [run them locally](pages/deploy/self_hosting.md).

**Secure** - Restrict access to your private apps with [token-based authentication](pages/deploy/security.md).

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
