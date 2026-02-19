---
icon: lucide/download
---

# Install

## Quick install

Use the install script or install via Cargo:

=== ":octicons-terminal-16: Shell"

    ```console
    $ curl -fsSL https://statespace.com/install.sh | sh
    ```

=== ":simple-rust: Cargo"

    ```console
    $ cargo install statespace
    ```

## Verify installation

Check that Statespace is installed correctly:

```console
$ statespace --version
statespace version 0.1.0
```

## Create your first app

Create a directory with a README.md file and serve it:

```bash
mkdir app
echo "# Hello, agent!" > app/README.md
statespace serve app/
```


> **Note**: Point your agent to [`http://127.0.0.1:8000`](http://127.0.0.1:8000) to check out your first app.