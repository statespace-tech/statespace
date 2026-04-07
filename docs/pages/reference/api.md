---
icon: lucide/globe
hide:
  - toc
---

# REST API

REST API for interacting with Statespace applications.

## <span class="http-method http-get">GET</span> `/{path}`

Read a file from the app's directory. Requesting `/` returns `API.md`.

<div class="grid" markdown>

<div markdown>

**Path parameters**

`path` (string, required)

: Path to file (e.g., `README.md`, `src/data.csv`, `data/logs.txt`).

**Query parameters**

`{key=value}` (string, optional)

: Environment variables injected into components.

**Headers**

`authorization` (string, optional)

  : Bearer token for authentication.

**Responses**

| Status | Description |
|--------|-------------|
| `200` | File content (`text/markdown`). |
| `400` | Invalid query parameters. |
| `404` | File not found. |
| `500` | Server error. |

</div>

<div markdown>

**Example**

```bash
curl -X GET \
  -H "Authorization: Bearer <TOKEN>" \
  "https://demo.statespace.app/page.md?name=Alice"
```

**Page** (`page.md`)

````markdown
# This is a Markdown page

```component
echo "You are talking to: $name"
```
````

**Example Response**

```markdown
# This is a Markdown page

You are talking to: Alice
```

</div>

</div>

## <span class="http-method http-post">POST</span> `/{path}`

Execute a tool declared in the page's frontmatter. Requesting `/` executes tools declared in `README.md`.

<div class="grid" markdown>

<div markdown>

**Path parameters**

`path` (string, required)

: Path to the page declaring the tool (e.g., `README.md`, `pages/search.md`).

**Request body (JSON)**

`command` (array, required)

: Command to execute as an array of strings (e.g., `["echo", "hello, world!"]`).

`env` (object, optional)

: Environment variables to pass to the tool (e.g., `{"USER": "john"}`).

**Headers**

`authorization` (string, optional)

  : Bearer token for authentication.

**Response (JSON)**

`data.stdout` (string)

: Standard output from the command.

`data.stderr` (string)

: Standard error from the command.

`data.returncode` (integer)

: Exit code of the command (`0` for success, non-zero if the command exited with an error).

**Responses**

| Status | Description |
|--------|-------------|
| `200` | Tool executed successfully. |
| `400` | Command not allowed or validation error. |
| `404` | Page not found. |
| `422` | Malformed request body. |
| `500` | Server error. |

</div>

<div markdown>

**Example**

```bash
curl -X POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <TOKEN>" \
  "https://demo.statespace.app/page.md" \
  -d '{
    "command": ["grep", "error"]
  }'
```

**Example Response**

```json
{
  "data": {
    "stdout": "logs/app.log:Connection error\n",
    "stderr": "",
    "returncode": 0
  }
}
```

**Example Error Response**

```json
{
  "error": "Command 'rm' not allowed by frontmatter of this page. See /AGENTS.md for API instructions."
}
```

</div>

</div>
