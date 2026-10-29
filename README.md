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

Statespace runs a commit of your app on a small share of live traffic,
beside a control, and records what happens.

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

An arm is a commit. You add it with the CLI. Statespace runs the image of that
commit for a share of the app's subjects. It runs a control beside it: a copy
of the image that live runs now. Everyone else gets live, which Statespace
never records.

```text
                    ┌─► live service   (90%)  untouched, never recorded
visitors ─► proxy ──┼─► control copy   ( 5%)  image of live, recorded
                    └─► arm a1b2c3d    ( 5%)  image of a1b2c3d, recorded
```

- An **app** is one running copy of a service, such as `shop-prod`. One proxy
  is in front of it.
- A **build** is a record "commit → image digest". Your CI makes it with
  `statespace-tech/action@v1`.
- The **live commit** is the commit of the build that live runs.
- The **control** and each **arm** are **units**. The proxy runs them on
  Docker, Cloud Run, or Kubernetes.

Statespace posts the evidence on the arm's commit as the commit status
`statespace/<app>`. The status tells where the arm is, never whether it is
good. People and agents decide.

## Set up

1. On [statespace.com](https://statespace.com), install the Statespace GitHub
   App. It reads code and metadata, reads members, and writes commit
   statuses.
2. In the folder of the app, write `statespace.toml` and get the CI step:

   ```shell
   ssp init
   ```

3. Add the step after the image build, in the workflow that builds every
   commit that can become an arm. The job needs `permissions: id-token: write`.

   ```yaml
   - uses: statespace-tech/action@v1
     with:
       image: ghcr.io/acme/shop@${{ steps.build.outputs.digest }}
       path: shop          # the folder of statespace.toml
       app: shop-prod      # optional: a build for one app only
   ```

4. Add an app for each running copy of the service. Run the command in the
   app's folder. It reads the repository from the Git remote `origin` and the
   folder from the nearest `statespace.toml`, and prints what it found.

   ```shell
   ssp app add shop-prod --note "shop, all traffic"
   ```

5. Add a runtime token, and run the proxy in front of the app with it:

   ```shell
   ssp token add --app shop-prod --note "Cloud Run proxy"
   ```

6. Add an arm:

   ```shell
   ssp arm add a1b2c3d --app shop-prod --share 0.05 --note "green button"
   ```

## Arms

- The commit is the arm's ID. Use a full SHA or a prefix of 7 or more
  characters.
- `ssp arm add` finds the build of the commit for the app and prints the
  build it chose. Without a build, it fails. Push the commit so that CI
  builds it.
- `--share` is 0.05 when you do not give it. Add an arm again to change its
  share. A new version starts.
- `ssp arm remove` ends the arm. Statespace keeps its evidence.
- An arm waits when `max_traffic` or `max_arms` is full, when the app has no
  live commit, or when a hold covers it. `ssp arm show` tells why.
- A new version of an arm starts when its share, the live commit, the
  settings, or a hold changes. Evidence restarts with each version.

## Stop and resume

A stop places a hold. The arm leaves traffic in a few seconds. A resume
removes exactly the hold it names.

```shell
ssp app stop shop-prod --note "checkout errors > 2%"
ssp arm stop a1b2c3d --app shop-prod
ssp app resume shop-prod
ssp arm resume a1b2c3d --app shop-prod --note "fixed in b2c3d4e"
```

When a hold on the app and a hold on the arm both cover an arm, `ssp app
resume` keeps the arm's hold and tells you. A resume never brings back a
removed arm.

## statespace.toml

The file is in the folder of the app's code. The settings that apply to an
app come from the `statespace.toml` in the build of its live commit. A change
takes effect when that build runs as live. The proxy has no flags for
settings.

```toml
subject = 'request.cookies.ssp_sid'
eligibility = 'request.headers["cf-ipcountry"] == "US"'
max_traffic = 0.2
max_arms = 5
arm_role = "write"
overrides = false
redact = ['\b\d{16}\b']

[[event]]
name = "signup"
match = 'request.method == "POST" && request.path == "/signup" && request.status < 300'

[[event]]
name = "fatal"
match = 'stderr.text.contains("FATAL")'

[[metric]]
label = "signup rate %"
value = '100.0 * events["signup"].subjects / subjects'

[app.shop-dev]
max_traffic = 1.0
overrides = true
```

| Key | Default | Meaning |
| --- | --- | --- |
| `subject` | `request.cookies.ssp_sid` | CEL that gives the subject ID. Without a string, the request goes to live |
| `eligibility` | everyone | CEL that tells who may enter. An error sends the request to live |
| `max_traffic` | `0.2` | The largest share of subjects in the arms and the control together |
| `max_arms` | `5` | The most arms that run at the same time. The control does not count |
| `arm_role` | `write` | The GitHub level needed to add and remove arms: `write`, `maintain`, or `admin` |
| `overrides` | `false` | Let testers pick a running arm. See [Overrides](#overrides) |
| `redact` | none | Regular expressions. The proxy replaces matches with `[redacted]` before data leaves the machine |
| `[[event]]` | none | A named count. See [Events](#events) |
| `[[metric]]` | none | A CEL formula over the totals of one side. The first metric is the headline |

A section `[app.<name>]` changes the settings of one app.

## Events

Each `[[event]]` matches one source:

| Source | Fields |
| --- | --- |
| `request` | `method`, `path`, `query`, `headers`, `cookies`, `status`, `ms` |
| `stdout`, `stderr` | `text`, and `json`: the line as a JSON object, or `{}` |

The proxy checks the rules for the control and the arms only. It checks a
request rule after the response, and an output rule for each line. A missing
field is no match. A request event has the subject; an output event has none.

## The proxy

```shell
ssp proxy docker      --app shop-dev  --image-file /etc/shop/image
ssp proxy cloudrun    --app shop-prod --service shop --region us-east1
ssp proxy kubernetes  --app shop-prod --deployment shop --namespace prod
```

Every subcommand takes these flags:

| Flag | Environment | Meaning |
| --- | --- | --- |
| `--app` | `STATESPACE_APP` | The app. It must exist |
| `--token` | `STATESPACE_TOKEN` | The runtime token of the app |
| `--listen` | `STATESPACE_LISTEN` | The public address. The default is `0.0.0.0:8080` |
| `--api` | `STATESPACE_API` | The Statespace API. The default is `https://api.statespace.com` |

For each request, the proxy:

1. Removes the headers `x-statespace-*` and `ssp-arm`, the parameter
   `ssp_arm`, and the cookie `ssp_arm`. The app never sees them.
2. Reads the subject, checks `eligibility`, and finds the subject's side:
   the first 53 bits of `SHA-256(salt:subject)` give a point in `[0, 1)`.
   Each arm and the control hold ranges of that space.
3. Sends the request to live, the control, or the arm. A unit that is not
   ready sends the request to live.
4. Adds `x-statespace-subject`. For the control and the arms, it also adds
   `x-statespace-side` (`control` or `arm`). For an arm, it adds
   `x-statespace-arm` with the full commit.
5. Marks the responses of the control and the arms `Cache-Control: private`.

With the default `subject`, every response sets the visitor cookie. It lasts
90 days and works in iframes on other sites:

```text
ssp_sid=<id>; Path=/; Max-Age=7776000; HttpOnly; Secure; SameSite=None; Partitioned
```

The proxy uses `SameSite=Lax` without `Secure` and `Partitioned` when
`X-Forwarded-Proto` is not `https`.

### Many instances

Every instance of the proxy routes. Only the instance that holds the app's
lease starts and stops units and reads their output. The instance renews the
lease every 10 seconds. When it stops, another instance gets the lease in 30
seconds or less.

### Docker

`ssp proxy docker` runs live, the control, and the arms as containers on the
machine. `--image-file` holds the image of live. When the file changes, the
proxy starts the new image, sends traffic to it when it is ready, and stops
the old container.

| Flag | Meaning |
| --- | --- |
| `--image-file` | The file that holds the live image |
| `--port` | The port of the app in the container. The default is the first `EXPOSE` port |
| `--health` | A path that must answer 200 before a container gets traffic. Without it, an open port is enough |
| `--volume`, `--memory`, `--cpus` | Container settings |

Containers get the proxy's environment, but never the variables that start
with `STATESPACE_` or `SSP_`. They also get `PORT`. The proxy labels each
container. After a crash, a new proxy adopts the containers that still run.
When the proxy stops, it stops all its containers.

### Cloud Run

`ssp proxy cloudrun` runs as its own Cloud Run service in front of the app's
service. The control and each arm are revisions of the app's service with 0%
traffic and a tag: `ssp-control` or `ssp-<first 12 characters of the
commit>`. A new revision copies the service's template and changes only the
image. Live is the image of the revision with the most traffic.

| Flag | Meaning |
| --- | --- |
| `--service` | The service that runs live |
| `--region` | The region of the service |
| `--project` | The project. The default is the project of the proxy |

The proxy uses the service account of its own service. Give that account the
role Cloud Run Developer on the app's service, and the role Service Account
User on the app's runtime service account. To read output, give it Logs
Viewer. The proxy sends an ID token in `X-Serverless-Authorization`, so give
it Cloud Run Invoker on the app's service too.

The first unit pins the service's traffic to the revision that has it now,
so that new revisions of units get no traffic. After you deploy a new live,
send the traffic to it. The tags of the units stay.

```shell
gcloud run deploy shop --image ghcr.io/acme/shop@sha256:3333…
gcloud run services update-traffic shop --to-latest
```

### Kubernetes

`ssp proxy kubernetes` runs as its own Deployment in front of the app. Live is
the app's Deployment and its Service. The control and each arm are copies of
the Deployment's pod spec with another image, named
`<deployment>-ssp-control` and `<deployment>-ssp-<commit 12>`. Each copy has a
Service and the label `statespace.com/app`. Its replicas are live's replicas
times its share, at least one. Kubernetes joins standard output and standard
error, so the proxy records all lines as `stdout`.

| Flag | Meaning |
| --- | --- |
| `--deployment` | The Deployment that runs live |
| `--namespace` | The namespace. The default is the proxy's own |
| `--service` | The Service of live. The default is the Deployment's name |
| `--container` | The app's container. The default is the first container |

[`examples/kubernetes`](examples/kubernetes) has the manifests.

## Overrides

With `overrides = true` for an app, a tester can open a running arm:

1. In a browser, add `?ssp_arm=<commit>` to a URL, such as
   `https://shop.example.com/?ssp_arm=a1b2c3d`. The proxy keeps the arm in the
   cookie `ssp_arm` for an hour, renewed on each request. A `GET` goes back to
   the same URL without the parameter.
2. To end it, open a URL with `?ssp_arm=` and no value.
3. An API client sends the header `ssp-arm: a1b2c3d` on each request.

The value is a full commit or a prefix of 7 or more characters that matches
one running arm. For another value, the proxy shows a short page with the
running arms. Statespace records these visits with the reason `override`.
They do not count as subjects.

## Recording

The proxy records the control and the arms, never live:

| Rows | One row per |
| --- | --- |
| `runs` | subject, version, and reason: `assigned`, `ineligible`, `eligibility-error`, `arm-unavailable`, or `override` |
| `requests` | request: method, path, query, status, time. Never headers or bodies |
| `stdout`, `stderr` | output line |
| `events` | match of an `[[event]]` |
| `lifecycle` | start, ready, stop, or failure of a unit |

The proxy applies `redact` before rows leave the machine. It buffers rows and
drops them before it slows the app.

## Credentials

The CLI uses the first credential that it finds:

1. `STATESPACE_TOKEN`.
2. The session of `ssp login`.
3. A GitHub user token in `GH_TOKEN` or `GITHUB_TOKEN` (`gho_`, `ghp_`,
   `ghu_`, or `github_pat_`). Installation tokens (`ghs_`) do not count.
4. In GitHub Actions, the job's OIDC token, with the audience `statespace`.
   The job needs `permissions: id-token: write`.

`ssp whoami` shows the credential that the CLI uses. Your permissions come
from GitHub:

| Level | Allows | GitHub |
| --- | --- | --- |
| `read` | see apps, arms, and evidence | read |
| `write` | stop and resume; add and remove arms when `arm_role = "write"` | write (triage: stop and resume only) |
| `maintain` | add and remove arms when `arm_role = "maintain"` | maintain |
| `admin` | add and remove apps and runtime tokens | admin |

Runtime tokens are the only Statespace tokens. A runtime token is for one
app. It can read the app's runtime configuration, report, hold the lease, and
send rows. It cannot change arms or read evidence.

## Commands

```text
ssp app    add | remove | list | show | stop | resume
ssp arm    add | remove | list | show | stop | resume   (--app)
ssp token  add | remove | list                         (--app)
ssp proxy  docker | cloudrun | kubernetes              (--app)
ssp init
ssp login | logout | whoami
```

- The object comes first: `ssp arm stop a1b2c3d --app shop-prod`.
- Each command that changes something takes `--note`: 1 to 200 characters on
  one line.
- With `--json`, each command prints one JSON document, except `ssp proxy`.
  Notes and warnings go to standard error. An error exits with status 1, as
  `{"error": "..."}` on standard error.
- No command asks a question, except `ssp login`.

## License

Apache-2.0
