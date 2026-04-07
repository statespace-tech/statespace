---
icon: lucide/plug
---

# API

Agents interact with Statespace apps over plain HTTP — `GET` to read a page, `POST` to run a tool.

!!! warning "Warning"

    Always sandbox agents when connecting to public apps. Agents may have access to sensitive local data (e.g. environment variables, credentials, files), and a malicious app could use prompt injection to exfiltrate it.

## Coding agents

Coding agents natively support HTTP requests — simply include the app URL in your prompt:

=== ":simple-claude: &nbsp; Claude Code"

    ```bash
    claude "Search the logs at https://demo-api.statespace.app for any errors"
    ```

=== ":simple-cursor: &nbsp; Cursor"

    ```bash
    agent "Search the logs at https://demo-api.statespace.app for any errors"
    ```

=== ":simple-githubcopilot: &nbsp; GitHub Copilot"

    ```bash
    copilot -p "Search the logs at https://demo-api.statespace.app for any errors"
    ```


## Custom agents

Custom agents need an HTTP request tool to interact with apps:

=== ":simple-python: &nbsp; Python"

    ```python
    import httpx

    @tool
    def http_request(url: str, method: str = "GET", body: dict = None) -> str:
        """
        Make HTTP requests to interact with Statespace apps.
        """
        response = httpx.request(method, 
                                url=url, 
                                json=body)
        return response.text
    ```

=== ":simple-typescript: &nbsp; TypeScript"

    ```typescript
    /**
     * Make HTTP requests to interact with Statespace apps.
     */
    async function http(url: string, method = "GET", body?: object) {
        const response = await fetch(url, {
            method,
            body: body ? JSON.stringify(body) : undefined,
            headers: body ? { "Content-Type": "application/json" } : undefined
        });
        return response.text();
    }
    ```

## Authentication

Private apps require an [access token](../deploy/security.md#access-tokens) — pass it directly to your coding agents:

```bash
claude "Use 'Bearer sk-xxx' to authenticate with https://myapp.statespace.app"
```

For custom agents, add it to the `Authorization` header in your HTTP requests:

```python
response = httpx.request("GET", url=url, headers={"Authorization": "Bearer <TOKEN>"})
```

!!! abstract "Work in progress"

    Custom agents are the most secure option since tokens are injected directly into HTTP requests and never pass through the agent. We're working on similar guarantees for coding agents.
