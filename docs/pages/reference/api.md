---
icon: lucide/globe
---

# REST API

REST API endpoints for interacting with running applications. All endpoints use your app's base URL (e.g., `https://example.statespace.app` or `http://127.0.0.1:800`).

## <span class="http-method http-get">GET</span> `/{path}`

Read a file from the application directory.

- For Markdown pages, [components](../develop/components.md) are executed and replaced inline before returning.
- For all other files (`.csv`, `.txt`, `.py`, etc.), the raw content is returned as-is.
- Do not pass secrets as query parameter — use global static [environment variables](../develop/components.md) instead.


<div class="grid" markdown>

<div markdown>

**Path parameters**

`path` (string, required)

: Path to file (e.g., `README.md`, `src/data.csv`, `data/logs.txt`).

**Query parameters**

`{key=value}` (string, optional)

: Environment variables injected into [components](../develop/components.md).

**Headers**

`authorization` (string, optional)

  : Bearer token for authentication.

**Response**

: File content.

</div>

<div markdown>

**Example**

```bash
curl -X GET \
  -H "Authorization: Bearer <TOKEN>" \
  "https://example.statespace.app/page.md?name=Alice"
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

## <span class="http-method http-post">POST</span> `/`

Execute a [tool](../develop/tools.md) defined in a Markdown page's frontmatter.

- Tools are global — any tool defined in any page can be called from the base URL.
- The command must match a tool's allowed pattern, or it will be rejected.

<div class="grid" markdown>

<div markdown>

**Request body (JSON)**

`command` (array, required)

: Command to execute as an array of strings (e.g., `["echo", "hello, world!"]`).

`env` (object, optional)

: Environment variables to pass to the tool (e.g., `{"USER": "john"}`).

**Headers**

`authorization` (string, optional)

  : Bearer token for authentication.

**Response (JSON)**

`stdout` (string)

: Standard output from the command.

`stderr` (string)

: Standard error from the command.

`returncode` (integer)

: Exit code (0 for success, non-zero for errors).

</div>

<div markdown>

**Example**

```bash
curl -X POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <TOKEN>" \
  "https://example.statespace.app" \
  -d '{
    "command": ["grep", "error"]
  }'
```

**Example Response**

```json
{
  "stdout": "logs/app.log:Connection error\n",
  "stderr": "",
  "returncode": 0
}
```

</div>

</div>
