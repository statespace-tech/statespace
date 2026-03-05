---
icon: lucide/bot
---

# Agents

Connect agents to your applications.

!!! warning "Warning"

    Always sandbox agents when connecting to public apps. Agents may have access to sensitive local data (e.g. environment variables, credentials, files), and a malicious app could use prompt injection to exfiltrate it.
## Coding agents

Coding agents natively support HTTP requests — simply include the app URL in your prompt:

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


Private apps require an [access token](../deploy/security.md#access-tokens). For coding agents, pass the token in your prompt:

```console
$ claude "Use 'Bearer sk-xxx' to authenticate with https://myapp.statespace.app"
```

For custom agents, add it to the `Authorization` header in your HTTP requests:

```python
response = httpx.request(method, url=url, json=body,
                         headers={"Authorization": "Bearer <token>"})
```

!!! abstract "Work in progress"

    Custom agents are more secure by default since tokens are injected directly into HTTP requests, bypassing the agent. We're working on similar support for coding agents.