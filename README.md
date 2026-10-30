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

Statespace runs commits of your app on small shares of live traffic, as the
arms of an experiment, and records what happens.

---

**Website:** [statespace.com](https://statespace.com)<br>
**Documentation:** [docs.statespace.com](https://docs.statespace.com)

---

## Install

Install the CLI on macOS or Linux.

```shell
curl -fsSL https://statespace.com/install.sh | bash
```

The command is `ssp`. `statespace` is the same program, so
`statespace experiment list` and `ssp experiment list` do the same thing.

Give this prompt to your coding agent:

```text
Read https://statespace.com/skill.md and follow it.
```

## How it works

An experiment compares commits of one app. Each arm is one commit with an
exact share of the app's subjects. The proxy runs the image of each arm
beside live. Everyone else gets live, which Statespace never records.

```text
                    ┌─► live service        (85%)  untouched, never recorded
visitors ─► proxy ──┼─► checkout:c0ffee1    ( 5%)  recorded
                    ├─► checkout:a1b2c3d    ( 5%)  recorded
                    └─► us-landing:b2c3d4e  ( 5%)  recorded
```

- An **app** is one running copy of a service, such as `shop-prod`. One
  proxy is in front of it.
- A **build** is a record "commit → image digest". Your CI makes it with
  `statespace-tech/action@v1`.
- The **live commit** is the commit of the build that live runs.
- An **experiment** is a table in `statespace.toml`, such as
  `[experiment.checkout]`. You set its arms with the CLI.
- The **reference** is the arm that you added first. To compare a change
  with what runs now, add the live commit as an arm.
- A **unit** is one running copy of one arm. The proxy runs units on Docker,
  Cloud Run, Kubernetes, or ECS Fargate.

Statespace posts the evidence on the commit of each arm as the commit status
`statespace/<app>/<experiment>`. The status tells where the arm is, never
whether it is good. People and agents decide.

## Set up

1. On [statespace.com](https://statespace.com), install the Statespace
   GitHub App. It reads code and metadata, reads members, and writes commit
   statuses.
2. In the folder of the app, write `statespace.toml`. Start from the file in
   [`skills/statespace/SKILL.md`](skills/statespace/SKILL.md). A coding
   agent can do this step.
3. Add the step after the image build, in the workflow that builds every
   commit that can become an arm. The job needs
   `permissions: id-token: write`.

   ```yaml
   - uses: statespace-tech/action@v1
     with:
       image: ghcr.io/acme/shop@${{ steps.build.outputs.digest }}
       path: shop          # the folder of statespace.toml
       app: shop-prod      # optional: a build for one app only
   ```

4. Add an app for each running copy of the service. Run the command in the
   app's folder. It reads the repository from the Git remote `origin` and
   the folder from the nearest `statespace.toml`, and prints what it found.

   ```shell
   ssp app add shop-prod --note "shop, all traffic"
   ```

5. Add a runtime token, and run the proxy in front of the app with it:

   ```shell
   ssp token add --app shop-prod --note "Cloud Run proxy"
   ```

6. Set the arms of an experiment, and start it:

   ```shell
   ssp experiment update checkout \
     --arm c0ffee1=0.05 --arm a1b2c3d=0.05
   ssp experiment start checkout --note "one-page checkout"
   ```

## Experiments

```shell
ssp experiment update checkout --arm e5f6a7b=0.04    # add
ssp experiment update checkout --arm c0ffee1=0.06    # change a share
ssp experiment update checkout --arm e5f6a7b=0       # remove
ssp experiment start checkout --note "one-page checkout"
ssp experiment show checkout
ssp experiment stop checkout --note "enough data"
```

`update` prints the change and the full list of arms:

```text
checkout · shop-prod · stopped
  − a1b2c3d   0.05
  + f6a7b8c   0.05
    c0ffee1   0.06   (reference)
  total 0.11 of max_share 0.2
```

- `--arm <commit>=<share>` changes only the arm that it names. The commit is
  a full SHA or a prefix of 7 or more characters. The share is a fraction,
  such as `0.05`. `5` and `5%` fail.
- `=0` removes an arm. Statespace keeps its evidence.
- Without `--arm`, `update` prints the arms.
- A commit needs a build for the app. Push the commit so that CI builds it.
- The shares of an experiment add up to `max_share` or less. The shares of
  all running experiments of an app add up to 1 or less.
- You cannot change a running experiment. Stop it, update it, and start it
  again.

`start` begins a new version. The command waits until each arm is ready and
prints each one. When an arm is not ready in 5 minutes, nothing changes, and
the command prints the reason:

```text
starting checkout on shop-prod
  c0ffee1   ready
  a1b2c3d   not ready: the container exits at start (exit 1)
checkout did not start. Nothing changed.
```

`stop` takes one or more names. The arms leave traffic in seconds. To stop
every experiment of an app, get the names with
`ssp experiment list --running`, then stop them.

A release that changes the `subject`, `eligibility`, `max_share`, or events
of a running experiment, or a shared event, stops the experiment. A change
of the code or of a metric stops nothing. `ssp experiment show` tells when
live changed since the start.

## statespace.toml

The file is in the folder of the app's code. The settings that apply to an
app come from the `statespace.toml` in the build of its live commit. A
change takes effect when that build runs as live. The proxy has no flags for
settings.

```toml
overrides = false
redact = ['\b\d{16}\b']

# Shared events and metrics: in every experiment.
[[event]]
name = "purchase"
match = '''
request.method == "POST"
  && request.path == "/checkout"
  && request.status < 300
'''

[[metric]]
label = "purchase rate %"
value = '100.0 * events["purchase"].subjects / subjects'

# Experiments, in priority order.
[experiment.checkout]
subject = 'request.headers["x-user-id"]'
max_share = 0.2

[[experiment.checkout.event]]
name = "coupon"
match = 'request.path == "/checkout/coupon"'

[experiment.us-landing]
eligibility = 'request.headers["cf-ipcountry"] == "US"'
max_share = 0.15

# Per-app sections replace and extend the shared settings.
[app.shop-dev]
overrides = true

[app.shop-dev.experiment.checkout]
max_share = 1.0
```

| Key | Default | Meaning |
| --- | --- | --- |
| `overrides` | `false` | Let testers pick an arm. See [Overrides](#overrides) |
| `redact` | none | Regular expressions. The proxy replaces matches with `[redacted]` before data leaves the machine |
| `[[event]]` | none | A shared event. See [Events](#events) |
| `[[metric]]` | none | A shared metric: a CEL formula over the totals of one arm |
| `[experiment.<name>]` | none | An experiment. Names have 1 to 40 lowercase letters, digits, and `-` |
| `[app.<name>]` | none | Settings for one app |

Keys of an experiment:

| Key | Default | Meaning |
| --- | --- | --- |
| `subject` | `request.cookies.ssp_sid` | CEL that gives the subject ID. Without a string, the experiment skips the request |
| `eligibility` | everyone | CEL that tells who may enter. An error skips the request and counts an eligibility error |
| `max_share` | `0.2` | The most that the shares of its arms add up to |
| `[[experiment.<name>.event]]` | none | Events of this experiment only |
| `[[experiment.<name>.metric]]` | none | Metrics of this experiment only |

The headline is the first metric of the experiment, or the first shared
metric when it has none.

## Events

Each event matches one source:

| Source | Fields |
| --- | --- |
| `request` | `method`, `path`, `query`, `headers`, `cookies`, `status`, `ms` |
| `stdout`, `stderr` | `text`, and `json`: the line as a JSON object, or `{}` |

The proxy checks the rules for the units only. A unit gets the shared events
and the events of its experiment. The proxy checks a request rule after the
response, and an output rule for each line. A missing field is no match. A
request event has the subject; an output event has none.

## The proxy

```shell
ssp proxy docker      --app shop-dev  --image-file /etc/shop/image
ssp proxy cloudrun    --app shop-prod --service shop --region us-east1
ssp proxy kubernetes  --app shop-prod --deployment shop --namespace prod
ssp proxy fargate     --app shop-prod --cluster shop --service shop \
                      --live-url http://internal-shop.example.com
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
2. Checks the running experiments in the order of `statespace.toml`. For
   each one, it reads the subject and finds the subject's point: the first
   53 bits of `SHA-256(salt:subject)`, in `[0, 1)`. When the point is in the
   range of one of its arms and the subject is eligible, that arm gets the
   request. Else the proxy checks the next experiment.
3. Sends the request to live when no experiment takes it. A unit that is not
   ready also sends the request to live.
4. Adds `x-statespace-subject`. For an arm, it also adds
   `x-statespace-experiment` and `x-statespace-arm` with the full commit.
5. Marks the responses of units `Cache-Control: private`.

The arms of all running experiments of an app hold different ranges. Thus
experiments with the same subject never share a subject. Experiments with
different subjects can claim the same request: the first in
`statespace.toml` gets it. Different apps have different salts, so their
splits are independent.

While a running experiment uses the default `subject`, every response sets
the visitor cookie. It lasts 90 days and works in iframes on other sites:

```text
ssp_sid=<id>; Path=/; Max-Age=7776000; HttpOnly; Secure;
  SameSite=None; Partitioned
```

The proxy uses `SameSite=Lax` without `Secure` and `Partitioned` when
`X-Forwarded-Proto` is not `https`.

### Many instances

Every instance of the proxy routes. Only the instance that holds the app's
lease starts and stops units and reads their output. It starts the units of
starting and running experiments. Only running experiments get requests.
The instance renews the lease every 10 seconds. When it stops, another
instance gets the lease in 30 seconds or less.

### Units

A unit is one arm: an experiment, a commit, and the image of the commit's
build. One commit in two experiments runs as two units. The name of a unit
has the experiment and the commit. When a platform limit makes a name too
long, the name is cut and ends with a short hash. The labels keep the full
experiment and commit.

### Docker

`ssp proxy docker` runs live and the units as containers on the machine.
`--image-file` holds the image of live. When the file changes, the proxy
starts the new image, sends traffic to it when it is ready, and stops the
old container.

| Flag | Meaning |
| --- | --- |
| `--image-file` | The file that holds the live image |
| `--port` | The port of the app in the container. The default is the first `EXPOSE` port |
| `--health` | A path that must answer 200 before a container gets traffic. Without it, an open port is enough |
| `--volume`, `--memory`, `--cpus` | Container settings |

Containers get the proxy's environment, but never the variables that start
with `STATESPACE_` or `SSP_`. They also get `PORT`. The proxy labels each
container with `statespace.com/experiment` and `statespace.com/commit`.
After a crash, a new proxy adopts the containers that still run. When the
proxy stops, it stops all its containers.

### Cloud Run

`ssp proxy cloudrun` runs as its own Cloud Run service in front of the
app's service. Each unit is a revision of the app's service with 0% traffic
and the tag `ssp-<experiment>-<commit 12>`. The tag and the service name
together have at most 46 characters. A new revision copies the service's
template and changes only the image. Live is the image of the revision with
the most traffic.

| Flag | Meaning |
| --- | --- |
| `--service` | The service that runs live |
| `--region` | The region of the service |
| `--project` | The project. The default is the project of the proxy |

The proxy uses the service account of its own service. Give that account
the role Cloud Run Developer on the app's service, and the role Service
Account User on the app's runtime service account. When the images are in
Artifact Registry, give it Artifact Registry Reader on their repository:
Cloud Run deploys a revision only when the caller can read its image. To
read output, give it Logs Viewer. The proxy sends an ID token in `X-Serverless-Authorization`, so
give it Cloud Run Invoker on the app's service too.

While the proxy makes the revision of a unit, it pins the service's traffic
to the revision that has it now. When the revision exists, the proxy gives
the service its own template and traffic again, and adds only the tag. Thus
a deploy of a new live gets all the traffic, as before, and the tags of the
units stay. The service's template keeps the name of live's revision.

The proxy uses an ID token for the service's URL. Cloud Run accepts it at
each tag URL of the service.

The proxy works between requests. Deploy it with CPU that is always
allocated and with one instance at minimum.

### Kubernetes

`ssp proxy kubernetes` runs as its own Deployment in front of the app. Live
is the app's Deployment and its Service. Each unit is a copy of the
Deployment's pod spec with another image, named
`<deployment>-ssp-<experiment>-<commit 12>`, at most 63 characters. Each
copy has a Service and the labels `statespace.com/app`,
`statespace.com/experiment`, and `statespace.com/commit`. Its replicas are
live's replicas times its share, at least one. Kubernetes joins standard
output and standard error, so the proxy records all lines as `stdout`.

| Flag | Meaning |
| --- | --- |
| `--deployment` | The Deployment that runs live |
| `--namespace` | The namespace. The default is the proxy's own |
| `--service` | The Service of live. The default is the Deployment's name |
| `--container` | The app's container. The default is the first container |

[`examples/kubernetes`](examples/kubernetes) has the manifests.

### ECS Fargate

`ssp proxy fargate` runs as its own ECS service in front of the app. Live
is an ECS service. Each unit is an ECS service in the same cluster, named
`<service>-ssp-<experiment>-<commit 12>`, at most 255 characters. It runs a
copy of live's task definition with another image. The unit's service
copies live's network, launch, and deployment settings, but not its load
balancers or service discovery. Thus a unit gets no live traffic. The task
definition and the service have the tags `statespace.com/app`,
`statespace.com/experiment`, and `statespace.com/commit`. The desired count
is live's desired count times the share, at least one.

| Flag | Meaning |
| --- | --- |
| `--cluster` | The cluster of live's service |
| `--service` | The service that runs live |
| `--region` | The region. The default is the region of the AWS configuration |
| `--live-url` | Where live traffic goes, such as live's internal load balancer. The default is live's first Service Connect name |
| `--container` | The app's container. The default is the first container with a port |
| `--unit-ip` | `private` (the default) or `public`: the address of a unit's task that the proxy uses |

Live is the image of the task definition of the service's primary
deployment. When the image has a tag, the proxy reports the digest that
live's tasks run. A unit is ready when one of its tasks runs its image and
is healthy. A container without a health check must accept a TCP
connection. The proxy sends to the IP address of a task, and reads output
from CloudWatch Logs. The `awslogs` driver joins standard output and
standard error, so the proxy records all lines as `stdout`. When a unit
stops, the proxy deletes its service, and deregisters and deletes its task
definitions.

The proxy uses the AWS default chain for credentials. On Fargate, this is
its task role. The role needs the ECS actions on the cluster,
`iam:PassRole` for live's task role and execution role, and
`logs:FilterLogEvents` on live's log group. Units get no `STATESPACE_*`
variable or secret.

[`examples/fargate`](examples/fargate) has the task definition, the service,
and the policy of the proxy.

## Overrides

With `overrides = true` for an app, a tester can open an arm of a running
experiment:

1. In a browser, add `?ssp_arm=<experiment>:<commit>` to a URL, such as
   `https://shop.example.com/?ssp_arm=checkout:a1b2c3d`. The proxy keeps the
   arm in the cookie `ssp_arm` for an hour, renewed on each request. A `GET`
   goes back to the same URL without the parameter.
2. To end it, open a URL with `?ssp_arm=` and no value.
3. An API client sends the header `ssp-arm: checkout:a1b2c3d` on each
   request.

The commit is full or a prefix of 7 or more characters. It must match one
arm of a running experiment. For another value, the proxy shows a short
page:

```text
checkout:a1b2c3d is not running on shop-prod. Running: checkout:c0ffee1,
checkout:f6a7b8c, us-landing:b2c3d4e
```

Statespace records these visits with the reason `override`. They count for
the experiment: subjects, events, and metrics. Only the split check leaves
them out. `ssp experiment show` prints the override link of each arm.

## Recording

The proxy records the units, never live:

| Rows | One row per |
| --- | --- |
| `runs` | subject, experiment, version, and reason: `assigned`, `arm-unavailable`, `override`, or `eligibility-error` |
| `requests` | request: method, path, query, status, time. Never headers or bodies |
| `stdout`, `stderr` | output line |
| `events` | match of an event |
| `lifecycle` | start, ready, stop, or failure of a unit |

Each row has the experiment, the arm, and the version. The proxy applies
`redact` before rows leave the machine. It buffers rows and drops them
before it slows the app.

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
| `read` | see apps, experiments, and evidence | read, triage |
| `write` | update, start, and stop experiments | write, maintain |
| `admin` | add and remove apps and runtime tokens | admin |

A workflow's OIDC token of the app's repository counts as `write`.

Runtime tokens are the only Statespace tokens. A runtime token is for one
app. It can read the app's runtime configuration, report, hold the lease,
and send rows. It cannot change experiments or read evidence.

## Commands

```text
ssp app         add | remove | list | show
ssp experiment  list | show | update | start | stop        (--app)
ssp token       add | remove | list                         (--app)
ssp proxy       docker | cloudrun | kubernetes | fargate    (--app)
ssp login | logout | whoami
```

- The object comes first: `ssp experiment stop checkout --app shop-prod`.
- You can leave out `--app` when the current folder has exactly one app.
  The command tells which app it used. `ssp proxy` always needs `--app`.
- Each command that changes something takes `--note`: 1 to 200 characters
  on one line.
- With `--json`, each command prints one JSON document, except `ssp proxy`.
  Notes and warnings go to standard error. An error exits with status 1, as
  `{"error": "..."}` on standard error.
- No command asks a question, except `ssp login`.

## License

Apache-2.0
