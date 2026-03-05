---
icon: lucide/cloud-upload
---

# Cloud deployment

Deploy your app to get a URL for agents.


!!! info "First time?"

    Public apps are available on all plans. Upgrade to [pro](https://statespace.com/pricing) for private apps. [Create a free account](https://statespace.com/auth/login) to get started.

## Deploy

Run `statespace deploy` to deploy your app:

```console
$ statespace deploy ./myapp
Creating 'myapp'...

  ID:  myapp
  URL:  https://myapp.statespace.app
  Token:  <your-access-token>

✓ Created 'myapp'
```

Apps can be **public** (anyone can access) or **private** (requires an [access token](security.md)):

```console
$ statespace deploy ./myapp --public
$ statespace deploy ./myapp --private
```

To access **private** apps, include the token in the `Authorization` header:

```console
$ curl -H "Authorization: Bearer <token>" https://myapp.statespace.app
```

You can manage deployed apps from the [CLI](../reference/cli.md#app-management):

```console
$ statespace app list
$ statespace app get <app-id>
$ statespace app delete <app-id>
```

## Dependencies

By default, apps come with standard Unix utilities like `ls`, `cat`, `grep`, `curl`, and `date`:


````yaml title="README.md"
---
tools:
  - [grep, -r, { }, logs/]
  - [cat, { }]
  - [curl, -s, { }]
---

# My App

```component
echo "Today's date: $(date)"
```
````


Include an optional `Dockerfile` to customize the environment for your apps:

```text hl_lines="3"
myapp/
├── README.md
├── Dockerfile  # optional
└── ...
```


Use `RUN` to install additional CLI binaries.

```dockerfile title="Dockerfile"
# Install PostgreSQL client for database queries
RUN apt-get update && apt-get install -y --no-install-recommends postgresql-client

# Install Python for custom scripts
RUN apt-get install -y --no-install-recommends python3

# Install jq for JSON processing
RUN apt-get install -y --no-install-recommends jq
```
