---
icon: lucide/globe
---

# REST API

REST API endpoints for interacting with running applications

## `GET /{path}`

<div class="grid" markdown>

<div markdown>

Read a file from the application directory.

**Path parameters**

`path` <span class="param-tag param-type">string</span> <span class="param-tag param-required">required</span>

: Path to file (e.g., `README.md` or `src/tools.md`)

**Headers**

`authorization`  <span class="param-tag param-type">string</span>

  : Bearer token for authentication

**Response**

: Raw file content

</div>

<div markdown>

**Example**

```bash
curl -X GET \
  -H "Authorization: Bearer token" \
  https://127.0.0.1:8000/README.md
```

**Example Response**

```yaml
---
tools:
  - [ls]
  - [cat]
---

# Instructions
You are an AI agent.
```

</div>

</div>

## `POST /{path}`

<div class="grid" markdown>

<div markdown>

Execute a tool defined in a Markdown file's frontmatter.

: Path to Markdown file containing the tool definition

**Request body (JSON)**

`command` <span class="param-tag param-type">array</span> <span class="param-tag param-required">required</span>

: Command to execute as an array of strings (e.g., `["echo", "hello"]`)

`env` <span class="param-tag param-type">object</span> <span class="param-tag param-optional">optional</span>

: Environment variables to pass to the command (e.g., `{"USER": "john"}`)

**Headers**

`authorization`  <span class="param-tag param-type">string</span>

  : Bearer token for authentication

**Response (JSON)**

`stdout` <span class="param-tag param-type">string</span>

: Standard output from the command

`stderr` <span class="param-tag param-type">string</span>

: Standard error from the command

`returncode` <span class="param-tag param-type">integer</span>

: Exit code (0 for success, non-zero for errors)

</div>

<div markdown>

**Example**

```bash
curl -X POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer token" \
  -d '{
    "command": ["echo", "hello"]
  }'
```

**Example Response**

```json
{
  "stdout": "hello\n",
  "stderr": "",
  "returncode": 0
}
```

</div>

</div>
