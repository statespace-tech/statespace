<br>

<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/header-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="assets/header-light.png">
    <img src="assets/header-light.png" alt="Statespace" width="520">
  </picture>
</div>

<div align="center">

<br>

[![CI](https://github.com/statespace-tech/statespace/actions/workflows/ci.yml/badge.svg)](https://github.com/statespace-tech/statespace/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/statespace?style=flat-square)](https://crates.io/crates/statespace)
[![License](https://img.shields.io/badge/license-Apache--2.0-007ec6?style=flat-square)](https://github.com/statespace-tech/statespace/blob/main/LICENSE)
[![Discord](https://img.shields.io/discord/1541944682727084143?label=Discord&logo=discord&logoColor=white&color=5865F2&style=flat-square)](https://discord.gg/qKEFpqG9mr)

</div>

Statespace lets you A/B test multiple versions of your app in production.

---

**Website:** [statespace.com](https://statespace.com)<br>
**Documentation:** [docs.statespace.com](https://docs.statespace.com)

---

## Install

Install the CLI on macOS or Linux.

```shell
curl -fsSL https://statespace.com/install.sh | bash
```

Give this prompt to your coding agent:

```text
Read https://statespace.com/skill.md and follow it.
```

## How it works

`ssp proxy` is a reverse proxy in front of your app. For each request, it finds the
subject and assigns the subject to an arm. Then it sends the request to one of these:

- **The default:** the version that you deployed. All subjects outside experiments get it.
- **Control:** the baseline arm of an experiment. It is a copy of the default's image.
  Statespace pins it when the experiment starts.
- **A variant:** a signed image of a branch.

```text
load balancer → ssp proxy :8080 → default            (your deployed image)
                                → control of price   (the default's image, pinned)
                                → lower-price@3      (a signed image of a branch)
```

On a machine with Docker, the proxy runs the default and each arm as containers. On
Kubernetes, the proxy runs beside the default in each pod. `ssp controller` runs the arms
as copies of the default's Deployment.

The proxy sends the subject to your app in the `x-statespace-subject` header. Your app
prints to standard output and standard error as usual. Statespace records each line that
an arm prints. A JSON line with a `subject` field belongs to the arm of that subject.

## Quickstart

Sign in, create an app, and create a signing key. Keep the private key in CI.

```shell
ssp login
ssp app create web
ssp keygen
```

Build the image of the branch as you build production. Push the image. Then sign and
publish it.

```shell
docker build -t ghcr.io/acme/web:feature-x . && docker push ghcr.io/acme/web:feature-x
ssp variant create feature-x --app web --image ghcr.io/acme/web:feature-x
```

Create an experiment that gives 5% of the subjects to the variant and 5% to control. Then
start it.

```shell
ssp experiment create checkout --app web --split feature-x=0.05 \
  --note "A one-page checkout can increase purchases."
ssp experiment start checkout
```

Create a runtime token. Then run your app behind `ssp proxy`.

```shell
ssp token create --name production --preset runtime   # store it as SSP_TOKEN
ssp proxy --app web --runtime docker --image ghcr.io/acme/web:main --trust-key "$PUBLIC_KEY"
```

Print a line where the outcome occurs.

```python
print(json.dumps({"event": "purchase", "subject": user_id, "amount": 42}))
```

Create a token that can read the app. Then compare the arms in SQL.

```shell
ssp token create --name analysis --preset analyst --app web
psql "$DATABASE_URL" -c "
  SELECT variant, count(DISTINCT subject_id) AS subjects,
         count(DISTINCT subject_id) FILTER (WHERE data->>'event' = 'purchase') AS buyers
  FROM web.logs WHERE experiment = 'checkout' GROUP BY variant"
```

Commands print text. Add `--json` to print one JSON document.

## Variants

A variant is a signed image of your app. Usually, you build it from a branch. `--image`
takes a tag or a digest. The CLI uses `docker` to change a tag to its digest. Thus, the
signature covers the exact image.

```shell
ssp variant create feature-x --app web --image ghcr.io/acme/web:feature-x
```

Proxies and controllers run only variants with a signature from a key that they trust.
Thus, a stolen Statespace token cannot put code in production. The signature covers the
image digest, the name and app of the variant, and its commit. The commit comes from CI,
such as `GITHUB_SHA`, or from Git `HEAD` in the current directory.

Each new digest makes a new version of the variant. The same digest does not.

```shell
ssp variant show feature-x --app web
```

## Experiments

`--split` gives each arm its share of all subjects of the app. Control gets the share of
the largest variant. To set a different share, use `control=SHARE`. Two running
experiments of an app never share a subject. Subjects outside all experiments get the
default.

```shell
ssp experiment create checkout --app web --split feature-x=0.05 --split feature-y=0.02 \
  --subject 'request.cookies.session' \
  --eligibility 'request.headers["cf-ipcountry"] == "US"'
```

`--subject` and `--eligibility` are CEL expressions. They can use the `method`, `path`,
`query`, `headers`, and `cookies` of the request. The default subject is
`request.cookies.ssp_sid`. The proxy sets this cookie for each new visitor. You cannot
change the subject after the experiment starts.

Changes go to a draft. The running version changes only when you start the draft. Then
Statespace pins the newest version of each variant and gives the experiment a new control
arm.

```shell
ssp experiment update checkout --split feature-x=0.05 --note "feature-y made latency worse"
ssp experiment start checkout --note "remove feature-y"
```

To send all subjects back to the default, stop the experiment. To ship a variant, merge its
branch and deploy it as usual. To roll back, start an earlier version.

```shell
ssp experiment stop checkout --note "feature-x won; merging it"
ssp experiment start checkout --version 1
```

## Proxies

On a machine with Docker:

```shell
ssp proxy --app web --runtime docker \
  --listen 0.0.0.0:8080 \
  --image-file /etc/web/image \
  --health /health \
  --trust-key "$PUBLIC_KEY" \
  --variant-env DATABASE_URL="$VARIANT_DATABASE_URL" \
  --redact 'sk_live_\w+'
```

On Kubernetes, beside the default in each pod:

```shell
ssp proxy --app web --runtime kubernetes --default http://localhost:8000
```

| Flag | Meaning |
| --- | --- |
| `--runtime` | `docker` runs the default and the arms. `kubernetes` only sends requests |
| `--image`, `--image-file` | The image of the default. On SIGHUP, the proxy reads `--image-file` again. It replaces the default when the new container is ready |
| `--default` | The address of the default, with `--runtime kubernetes` |
| `--health` | A path that must answer 200 before a container gets requests |
| `--trust-key` | A public key. Proxies run variants that this key signed. Repeat it to replace keys |
| `--max-variants` | The maximum number of arms that run at the same time, control arms included. The default is 5 |
| `--variant-env` | An environment variable for the arms. The default does not get it |
| `--redact` | A regular expression. The proxy replaces its matches in recorded lines before they leave the machine |
| `--port`, `--volume`, `--memory`, `--cpus` | Container settings, with `--runtime docker`. The default port is the `EXPOSE` port of the image |
| `--stale-after` | How many seconds the proxy uses its last configuration when it cannot reach Statespace. Then all subjects get the default. The default is 300 |

On Docker, containers get the environment of the proxy. They do not get the settings of
the proxy or the variables that describe the machine. They also get `PORT`,
`STATESPACE_APP`, `STATESPACE_VARIANT`, `STATESPACE_EXPERIMENT`, and
`STATESPACE_VARIANT_VERSION`. Requests have the headers `x-statespace-subject`,
`x-statespace-variant`, and `x-statespace-experiment`. Clients cannot set these headers.

When an arm starts or fails, its subjects get the default. A control arm runs only an image
that the proxy ran as its default. Thus, Statespace cannot choose what runs. Arms record
each line that they print, each request, lifecycle events, CPU and memory, and HTTP
summaries for each minute. Statespace does not record the default.

Arms use the same database, cache, and queues as the default. Use `--variant-env` to give
them a database user without schema permissions. Turn off schedulers and workers that run
in your web process. Then nothing runs two times.

## Kubernetes

`ssp controller` runs the arms of the app's experiments. Each arm is a copy of the
default's Deployment with the image of the arm. The controller reports the address of each
arm, so the proxies can send requests to it. It also records each line that the arms
print.

```shell
ssp controller --app web --deployment web --trust-key "$PUBLIC_KEY"
```

[`examples/kubernetes`](examples/kubernetes) has the manifests: the sidecar, the
controller, and its permissions. [`examples/fastapi`](examples/fastapi) is a full app: a
FastAPI shop with a GitHub Actions workflow and a systemd unit. The workflow publishes the
image of each branch as a variant. The unit runs the proxy on a machine with Docker.

## Data

Each app has a PostgreSQL schema of views: `logs`, `runs`, `output`, `requests`,
`metrics`, `lifecycle`, `notes`, `hosts`, `arms`, and `sample_ratio`. Tokens that can read
all apps can also read `account.audit`.

```shell
ssp token create --name analysis --preset analyst --app web
psql "$DATABASE_URL" -c "SELECT * FROM web.sample_ratio WHERE mismatch"
```

To erase the data of one subject from an app:

```shell
ssp app erase-subject web user-42
```

## Tokens

A token opens the API. With `query:read`, it also opens PostgreSQL. Presets give the usual
scopes.

| Preset | For |
| --- | --- |
| `runtime` | `ssp proxy`: read the configuration of the app and send events |
| `controller` | `ssp controller`: the `runtime` scopes, and report the addresses of arms |
| `publisher` | CI: publish variants and run experiments |
| `analyst` | SQL: read runs, output, and requests |
| `admin` | All scopes, for agents that do all of the work |

```shell
ssp token create --name agent --preset admin --expires-in-days 30
ssp token list          # tokens and sign-in sessions
ssp whoami              # the credential in use and its scopes
ssp logout --all        # end all sessions
```

## License

Apache-2.0
