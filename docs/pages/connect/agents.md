---
icon: lucide/bot
---

# Agents

Any agent with HTTP request capabilities can interact with Statespace apps.

## Coding agents

Coding agents can make HTTP requests natively, so include the app URL in your prompt:

=== ":simple-claude: &nbsp; Claude Code"

    ```console
    $ claude "Multiply the random number in https://demo.app.statespace.com by 256"
    ```

=== ":simple-githubcopilot: &nbsp; GitHub Copilot"

    ```console
    $ copilot -p "Multiply the random number in https://demo.app.statespace.com by 256"
    ```

=== ":simple-cursor: &nbsp; Cursor"

    ```console
    $ agent "Multiply the random number in https://demo.app.statespace.com by 256"
    ```

!!! warning "Warning"

    Avoid connecting coding agents to public or untrusted apps. Coding agents have access to sensitive data on your local machine (environment variables, credentials, files). A malicious app could use prompt injection to trick the agent into leaking secrets. Always sandbox or restrict agents when connecting to public apps.

## Custom agents

Custom agents need a simple HTTP request tool to interact with Statespace apps:

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
    async function httpRequest(url: string, method = "GET", body?: object): Promise<string> {
        const response = await fetch(url, {
            method,
            body: body ? JSON.stringify(body) : undefined,
            headers: body ? { "Content-Type": "application/json" } : undefined
        });
        return response.text();
    }
    ```

> **Note**: Once your agent can make HTTP requests, include the app URL in prompts or instructions.

## Authentication

For apps protected with [access tokens](../deploy/security.md#access-tokens), include the `Authorization` header:

```bash
curl -H "Authorization: Bearer <token>" https://myapp.statespace.app
```