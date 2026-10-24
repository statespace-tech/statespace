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

Statespace runs experiments from pull requests. A branch named `exp/<name>` is
an experiment with one arm. When GitHub says that the branch's pull request can
merge, the arm runs the latest commit for a share of your users. Each arm is
compared with a control: the default that its branch started from. To compare
more arms in one experiment, name the branches `exp/<experiment>/<arm>`.

```text
exp/pricing            → experiment pricing, one arm
exp/checkout/one-page  ┐
exp/checkout/two-step  ┘→ experiment checkout, two arms
```

`ssp proxy` is a reverse proxy in front of your app. For each request, it finds
the subject and sends the request to one of these:

- **The default:** the version that you deployed. All subjects outside experiments get it.
- **A control:** a copy of the default that an arm's branch started from.
- **An arm:** the image of a commit of an `exp/` branch.

```text
load balancer → ssp proxy :8080
  ├→ default                (your deployed image)
  ├→ control of checkout    (the default of the base)
  └→ one-page@d4e5f6        (an image of the branch)
```

The proxy sends the subject to your app in the `x-statespace-subject` header. Your
app prints to standard output and standard error as usual. Statespace records each
line that an arm prints. A JSON line with a `subject` field belongs to the arm of
that subject.

## Set up

1. On [statespace.com](https://statespace.com), select **Connect GitHub**, and
   install the Statespace GitHub App on your repositories. The same screen signs
   you in. The GitHub organization or user becomes your workspace. Teammates
   join when they sign in with GitHub.

2. Merge the "Set up Statespace" pull request that the App opens. It adds
   `statespace.toml` and `.github/workflows/statespace.yml`. The workflow builds
   the `Dockerfile` nearest the root on each push to an `exp/` branch, and
   records the image. To build another way, such as with ko, Jib, or
   buildpacks, replace the build step. The file sets `max_traffic = 0.2`.

3. Add this step after the build in the workflow that builds production. It
   records each production image, so that Statespace knows the commit of the
   default that your proxies run. The App does not change deploy workflows.

   ```yaml
   - uses: statespace-tech/action@v1
     with:
       image: ghcr.io/acme/web@${{ steps.build.outputs.digest }}
   ```

4. Run your app behind `ssp proxy`, with a runtime token for the app. The proxy
   takes the app from the token.

   ```shell
   # Store the token as SSP_TOKEN where the proxy runs.
   ssp token create --name production --preset runtime --app web
   ssp proxy --image ghcr.io/acme/web:main --health /health
   ```

5. In branch protection for the default branch, require one approval. Then a
   second person approves each commit before users see it.

Print a line where the outcome occurs.

```python
print(json.dumps({"event": "purchase", "subject": user_id, "amount": 42}))
```

## Run an experiment

Push a branch and open a pull request.

```shell
git switch -c exp/one-page-checkout
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
| Has a hold from `ssp stop` | Stops, until `ssp resume` |
| Merged | The experiment ends. Its other arms run again after you update them with the default branch |
| Closed | The arm is removed |

The `Statespace` check on the pull request tells what runs and why an arm waits,
such as `Running at 10% (+10% control), 20% in the experiment`. It fails only
when CI could not record a commit, so a wait never blocks a merge. While the arm
runs, the check shows the evidence of the arm and its control: subjects,
requests, errors, p90 latency, the events they printed, and whether the split
matches the shares.

### Stop and resume

An arm runs when GitHub says that its pull request can merge, and no hold in
Statespace covers it. In an emergency, `ssp stop` places a hold. The arms stop
at once, and their subjects get the default. The pull requests do not change,
and the stop does not need GitHub. A hold also covers the arms that start
later, until `ssp resume` removes it.

```shell
# Every experiment of the app web
ssp stop web --reason "checkout errors > 2%"
# One experiment of the app
ssp stop web/price
# One arm, by its branch
ssp stop exp/price/lower
# The arms of one pull request
ssp stop https://github.com/acme/web/pull/418
# Show the arms and the holds
ssp status
# Remove the hold on the app, and only that hold
ssp resume web
```

`ssp resume` removes exactly the hold on its target. For example, at 02:00
an alert stops `web`, and at 02:10 you stop `exp/price/lower`. At 03:00,
`ssp resume web` runs the other arms again. `lower` stays stopped until
`ssp resume exp/price/lower`.

While a hold covers an arm, its `Statespace` check fails, such as
`Stopped by safranchik with ssp stop · checkout errors > 2%`. If the
repository requires the check, the pull request cannot merge until you resume
it. A push does not remove a hold. A draft pull request still stops its arm:
that is GitHub's "no".

On the dashboard, each arm, experiment, and app has **Stop** and **Resume**.
Scripts and alerts use `POST /v1/stop` and `POST /v1/resume` with a token that
has the `experiments:write` scope. A person needs the triage role on the
repository, as on GitHub. If a target fails, `ssp` names it and exits with
status 1.

### Open an arm yourself

A proxy with `--overrides` lets a request name the running arm that serves
it. Use it on staging, or on production to look at an arm.

1. In a browser, add `?ssp_arm=<branch>` to any URL of the proxy, such as
   `https://staging.acme.com/?ssp_arm=exp/price/lower`. The proxy keeps the
   arm in a cookie for an hour, renewed on each request, and opens the URL
   without the parameter.
2. To end it, open a URL with `?ssp_arm=` and no value.
3. An API client sends the header on each request:

   ```shell
   curl -X POST https://api.acme.com/orders \
     -H 'ssp-arm: exp/price/lower'
   ```

Only running arms open. For another branch, the proxy shows the arms that
run. Statespace records these visits as runs with the reason `override`. The
subject counts and the split test of the check do not include them; the
events that the arm prints do. Your app never sees the parameter, the header,
or the cookie. Without `--overrides`, the proxy ignores them.

Branch names of public repositories are public. With `--overrides`, anyone
who can reach the proxy can open a running arm. Keep staging behind a VPN,
basic auth, or an IP allowlist.

Create a token that can read the app. Then compare the arms in SQL.

```shell
ssp token create --name analysis --preset analyst --app web
psql "$DATABASE_URL" -c "
  SELECT arm,
         count(DISTINCT subject_id) AS subjects,
         count(DISTINCT subject_id)
           FILTER (WHERE data->>'event' = 'purchase') AS buyers
  FROM web.logs
  WHERE experiment = 'one-page-checkout'
  GROUP BY arm"
```

Commands print text. Add `--json` to print one JSON document.

## statespace.toml

The file is at the root of the repository, or in the folder of a service. Each
branch reads its own copy. All settings are in this file, so they change by
commits, with review and history.

```toml
app = "web"
share = 0.05
subject = 'request.headers["x-user-id"]'
eligibility = 'request.headers["cf-ipcountry"] == "US"'
max_traffic = 0.2
```

| Key | Read from | Meaning |
| --- | --- | --- |
| `app` | Each branch | The app. Its first build creates it |
| `share` | The `exp/` branch | The share of all the app's subjects that the arm gets. Its control gets the share of the largest arm with the same base |
| `subject` | The `exp/` branch | A CEL expression that gives the subject. The default is `request.cookies.ssp_sid`, a cookie the proxy sets for each new visitor |
| `eligibility` | The `exp/` branch | A CEL expression that tells who may enter. Omit it to let everyone in |
| `max_traffic` | The default branch | The share of the app's subjects that experiments may hold, controls included. The default is 1 |
| `output_retention_days` | The default branch | The days to keep the lines that arms print, 1 to 365. The default is 7 |

CEL expressions can use the `method`, `path`, `query`, `headers`, and `cookies`
of the request. All arms of an experiment must have the same `subject` and
`eligibility`. To check the file and see the plan without recording anything,
give an image that is in the registry:

```shell
ssp apply --dry-run --image ghcr.io/acme/web:latest
```

On the dashboard, **Edit** commits the change as the Statespace App, with you as
co-author. It commits arm settings to the arm's branch. It opens a pull request
for app settings, which take effect when it merges.

## Fair comparisons

- An arm runs only when its base, the commit where its branch left the default
  branch, ran as the default. Then the arm and its control differ only by the
  branch's change. With continuous deployment, a new branch waits for some
  minutes at most.
- Each base has its own control. When you update an arm with the default branch,
  it gets a control of the new base.
- Arms keep their subjects across commits. Two experiments of an app never share
  a subject.
- `max_traffic` limits the share of subjects in experiments, controls
  included. Arms that do not fit wait, first come, first served.

## Provenance

CI proves each image with its GitHub OIDC token. The token names the repository,
the workflow, and the commit, so CI keeps no Statespace secret. Proxies and
controllers run only the images that Statespace recorded. A stolen Statespace
token cannot record images or start arms; only a pull request that can merge
starts an arm.

## Proxies

On a machine with Docker, the proxy runs the default and the arms:

```shell
ssp proxy --listen 0.0.0.0:8080 \
  --image-file /etc/web/image \
  --health /health \
  --arm-env DATABASE_URL="$ARM_DATABASE_URL" \
  --redact 'sk_live_\w+'
```

On Kubernetes, or on any platform that runs the arms for it, the proxy only
routes:

```shell
ssp proxy --default http://localhost:8000
```

The proxy takes the app from a runtime token for one app. With a token for
every app, pass `--app web`.

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
| `--overrides` | Let a request name its running arm with `?ssp_arm=<branch>` or the header `ssp-arm`. See [Open an arm yourself](#open-an-arm-yourself) |

Containers get the environment of the proxy, without the settings of the proxy
or the variables that describe the machine. They also get `PORT`,
`STATESPACE_APP`, `STATESPACE_ARM`, `STATESPACE_COMMIT`, and
`STATESPACE_EXPERIMENT`. Requests have the headers `x-statespace-subject`,
`x-statespace-arm`, and `x-statespace-experiment`. Clients cannot set these
headers. Responses that an arm serves are `Cache-Control: private`, so a shared
cache never gives one subject's arm to another.

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
ssp controller kubernetes --deployment web
```

The controller and the proxies can use the same runtime token.

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

To erase the data of one subject from an app, for a privacy request, open the
app on the dashboard and select **Erase a subject**. Or use the API:

```shell
curl -X DELETE -H "Authorization: Bearer $SSP_TOKEN" \
  https://api.statespace.com/v1/apps/web/subjects/user-42
```

## Commands

| Command | Use |
| --- | --- |
| `ssp status [<target>...]` | Show what runs, and why, and each experiment's share of the app's subjects. Without a target: the arm of the `exp/` branch you are on, else the app or repository you are in |
| `ssp stop <target>... [--reason <text>]` | Stop arms at once with a hold in Statespace. Pull requests do not change |
| `ssp resume <target>...` | Remove the hold on each target, and only that hold |
| `ssp apply` | In CI: record the image of the job's commit. `statespace-tech/action` runs it. `--dry-run` checks `statespace.toml` anywhere |
| `ssp token create`, `list`, `revoke` | Tokens for proxies, controllers, agents, and SQL |
| `ssp login`, `logout`, `whoami` | Sign in with GitHub, and show your workspaces |
| `ssp proxy`, `ssp controller kubernetes` | Run on your servers |

A target is an app (`web`), an experiment (`web/price`), a branch
(`exp/price/lower`), a pull request URL, a repository URL, or a path such as
`.`: the app of the nearest `statespace.toml`. To delete an app, use the
dashboard.

## Workspaces

A workspace is the GitHub organization or user that installed the App. People
join every workspace whose installation they can access, and do what their role
on each repository allows: read sees, triage stops and resumes, write edits, and
admins of every repository manage tokens. `ssp status`, `stop`, and `resume` use
the workspace of their target's owner. Pass `--workspace acme`, or set
`STATESPACE_WORKSPACE`, to choose another.

## Tokens

A token opens the API. With `query:read`, it also opens PostgreSQL. Presets give
the usual scopes.

| Preset | For |
| --- | --- |
| `runtime` | `ssp proxy` and `ssp controller`: read the configuration of the app, send events, and report the addresses of arms. With `--app`, the token is for one app |
| `analyst` | SQL: read runs, output, and requests |
| `admin` | All scopes, for agents that do all of the work |

## License

Apache-2.0
