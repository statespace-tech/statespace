---
icon: lucide/globe
---

# REST API

REST API endpoints for interacting with running applications. All endpoints use your app's base URL (e.g., `https://demo.statespace.app` or `http://127.0.0.1:8000`).

## <span class="http-method http-get">GET</span> `/{path}`

Read a file from the app's directory.

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

**Response**

: File content.

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

Execute a tool.

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
  "https://demo.statespace.app/page.md" \
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
