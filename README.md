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

Statespace runs experiments from pull requests. A branch named
`exp/<experiment>/<arm>` is an arm of the experiment. When GitHub says that the
branch's pull request can merge, the arm runs the latest commit for a share of
your users. Each arm is compared with a control: the default that its branch
started from.

```text
exp/checkout/one-page  ┐
exp/checkout/two-step  ┘→ experiment "checkout": arms one-page and two-step, one control
exp/pricing            → experiment "pricing": one arm
```

`ssp proxy` is a reverse proxy in front of your app. For each request, it finds
the subject and sends the request to one of these:

- **The default:** the version that you deployed. All subjects outside experiments get it.
- **A control:** a copy of the default that an arm's branch started from.
- **An arm:** the image of a commit of an `exp/` branch.

```text
load balancer → ssp proxy :8080 → default                (your deployed image)
                                → control of checkout    (the default the branch started from)
                                → one-page@d4e5f6        (an image of the branch)
```

The proxy sends the subject to your app in the `x-statespace-subject` header. Your
app prints to standard output and standard error as usual. Statespace records each
line that an arm prints. A JSON line with a `subject` field belongs to the arm of
that subject.

## Set up

1. Sign in, and install the Statespace GitHub App on your repository.

   ```shell
   ssp login
   ssp repo connect
   ```

2. In the repository, write `statespace.toml` and the CI workflow.

   ```shell
   ssp init github --app web
   ```

   `ssp init github` also prints a step for the workflow that builds production.
   Add it after the build. The step records each production image, so that
   Statespace knows the commit of the default that your proxies run.

3. Run your app behind `ssp proxy`, with a runtime token.

   ```shell
   ssp token create --name production --preset runtime   # store it as SSP_TOKEN
   ssp proxy --app web --image ghcr.io/acme/web:main --health /health
   ```

4. In branch protection for the default branch, require one approval. Then a
   second person approves each commit before users see it.

Print a line where the outcome occurs.

```python
print(json.dumps({"event": "purchase", "subject": user_id, "amount": 42}))
```

## Run an experiment

Push a branch and open a pull request.

```shell
git switch -c exp/checkout/one-page
# change the code, then commit and push
gh pr create --fill
```

CI builds the image of each commit and runs `ssp apply`, which records it.
GitHub's own rules decide what runs:

| The pull request | The arm |
| --- | --- |
| Can merge | Runs the latest commit |
| Never could merge | Does not run |
| Could merge, and then could not | Runs the last commit that could merge. A warning shows after 7 days |
| Is a draft, or a reviewer requested changes | Stops |
| Merged | The experiment ends. Its other arms run again after you update them with the default branch |
| Closed | The arm is removed |

The `statespace/arm` check on the pull request tells what runs and why an arm
waits. It never fails, so it never blocks a merge. To stop an arm, make its pull
request a draft. In an emergency, `ssp stop checkout` stops all arms at once.
They run again after a new commit, or after their pull request is marked as a
draft and then ready.

Create a token that can read the app. Then compare the arms in SQL.

```shell
ssp token create --name analysis --preset analyst --app web
psql "$DATABASE_URL" -c "
  SELECT arm, count(DISTINCT subject_id) AS subjects,
         count(DISTINCT subject_id) FILTER (WHERE data->>'event' = 'purchase') AS buyers
  FROM web.logs WHERE experiment = 'checkout' GROUP BY arm"
```

Commands print text. Add `--json` to print one JSON document.

## statespace.toml

The file is at the root of the repository. Each branch reads its own copy.

```toml
app = "web"
share = 0.05
subject = 'request.headers["x-user-id"]'
eligibility = 'request.headers["cf-ipcountry"] == "US"'
```

| Key | Meaning |
| --- | --- |
| `app` | The app. Its first build creates it |
| `share` | The share of all the app's subjects that the arm gets. Its control gets the share of the largest arm with the same base |
| `subject` | A CEL expression that gives the subject. The default is `request.cookies.ssp_sid`, a cookie the proxy sets for each new visitor |
| `eligibility` | A CEL expression that tells who may enter. Omit it to let everyone in |

CEL expressions can use the `method`, `path`, `query`, `headers`, and `cookies`
of the request. All arms of an experiment must have the same `subject` and
`eligibility`. To test the file and a request:

```shell
ssp check --path /checkout --header cf-ipcountry=US --cookie ssp_sid=u_1
```

On the default branch, Statespace reads only `app`.

## Fair comparisons

- An arm runs only when its base, the commit where its branch left the default
  branch, ran as the default. Then the arm and its control differ only by the
  branch's change. With continuous deployment, a new branch waits for some
  minutes at most.
- Each base has its own control. When you update an arm with the default branch,
  it gets a control of the new base.
- Arms keep their subjects across commits. Two experiments of an app never share
  a subject.
- `ssp app set web --max-traffic 0.2` limits the share of subjects in
  experiments, controls included. Arms that do not fit wait, first come, first
  served.

## Provenance

CI proves each image with its GitHub OIDC token. The token names the repository,
the workflow, and the commit, so CI keeps no Statespace secret. Proxies and
controllers run only the images that Statespace recorded. A stolen Statespace
token cannot record images or start arms; only a pull request that can merge
starts an arm.

## Proxies

On a machine with Docker, the proxy runs the default and the arms:

```shell
ssp proxy --app web \
  --listen 0.0.0.0:8080 \
  --image-file /etc/web/image \
  --health /health \
  --arm-env DATABASE_URL="$ARM_DATABASE_URL" \
  --redact 'sk_live_\w+'
```

On Kubernetes, or on any platform that runs the arms for it, the proxy only
routes:

```shell
ssp proxy --app web --default http://localhost:8000
```

| Flag | Meaning |
| --- | --- |
| `--image`, `--image-file` | The image of the default. The proxy runs it and the arms as Docker containers. On SIGHUP, it reads `--image-file` again and replaces the default when the new container is ready |
| `--default` | The address of the default. The proxy only routes; a controller runs the arms |
| `--health` | A path that must answer 200 before a container gets requests |
| `--max-arms` | The maximum number of arms that run at the same time, controls included. The default is 5 |
| `--arm-env` | An environment variable for the arms. The default does not get it |
| `--redact` | A regular expression. The proxy replaces its matches in recorded lines before they leave the machine |
| `--port`, `--volume`, `--memory`, `--cpus` | Container settings, with `--image`. The default port is the `EXPOSE` port of the image |
| `--stale-after` | How many seconds the proxy uses its last configuration when it cannot reach Statespace. Then all subjects get the default. The default is 300 |

Containers get the environment of the proxy, without the settings of the proxy
or the variables that describe the machine. They also get `PORT`,
`STATESPACE_APP`, `STATESPACE_ARM`, `STATESPACE_COMMIT`, and
`STATESPACE_EXPERIMENT`. Requests have the headers `x-statespace-subject`,
`x-statespace-arm`, and `x-statespace-experiment`. Clients cannot set these
headers.

When an arm starts or fails, its subjects get the default. Arms record each line
that they print, each request, lifecycle events, CPU and memory, and HTTP
summaries for each minute. Statespace does not record the default.

Arms use the same database, cache, and queues as the default. Use `--arm-env`
to give them a database user without schema permissions. Turn off schedulers and
workers that run in your web process. Then nothing runs two times.

## Kubernetes

`ssp controller kubernetes` runs each arm and control as a copy of the default's
Deployment with the image of the arm. It reports the address of each arm, so the
proxies can send requests to it. It also records each line that the arms print.

```shell
ssp controller kubernetes --app web --deployment web
```

[`examples/kubernetes`](examples/kubernetes) has the manifests: the sidecar, the
controller, and its permissions. [`examples/fastapi`](examples/fastapi) is a full
app: a FastAPI shop with its workflows and a systemd unit for the proxy.

## Data

Each app has a PostgreSQL schema of views: `logs`, `runs`, `output`, `requests`,
`metrics`, `lifecycle`, `hosts`, `arms`, `pull_requests`, and `sample_ratio`.
Tokens that can read all apps can also read `account.audit`.

```shell
ssp token create --name analysis --preset analyst --app web
psql "$DATABASE_URL" -c "SELECT * FROM web.sample_ratio WHERE mismatch"
```

To erase the data of one subject from an app:

```shell
ssp app erase-subject web user-42
```

## Commands

| Command | Use |
| --- | --- |
| `ssp init github` | Write `statespace.toml` and the CI workflow |
| `ssp check` | Check `statespace.toml`, and test a request against its rules |
| `ssp apply` | In CI: record the image of the job's commit |
| `ssp status [experiment]` | Show experiments, their arms, and their pull requests |
| `ssp stop <experiment>[/<arm>]` | Stop arms at once, until a new commit or GitHub resumes them |
| `ssp repo connect`, `ssp repo list` | Install the GitHub App, and list its repositories |
| `ssp app list`, `show`, `set`, `erase-subject`, `delete` | Show and change apps |
| `ssp token create`, `list`, `revoke` | Tokens for proxies, controllers, agents, and SQL |
| `ssp login`, `logout`, `whoami` | Sign in with GitHub |
| `ssp proxy`, `ssp controller kubernetes` | Run on your servers |

## Tokens

A token opens the API. With `query:read`, it also opens PostgreSQL. Presets give
the usual scopes.

| Preset | For |
| --- | --- |
| `runtime` | `ssp proxy`: read the configuration of the app and send events |
| `controller` | `ssp controller`: the `runtime` scopes, and report the addresses of arms |
| `analyst` | SQL: read runs, output, and requests |
| `admin` | All scopes, for agents that do all of the work |

## License

Apache-2.0
