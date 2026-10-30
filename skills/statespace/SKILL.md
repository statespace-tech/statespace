---
name: statespace
description: Run commits of an app as arms of an experiment on shares of live traffic with Statespace, and read the evidence. Use when asked to set up Statespace, test a code change on live traffic, compare a commit with production, or read experiment results.
---

# Statespace

Statespace runs commits of an app on small shares of live traffic and records
what happens.

- An **app** is one running copy of a service, such as `shop-prod`. A proxy is
  in front of it.
- An **experiment** compares commits in one app. Its table in
  `statespace.toml` defines it, such as `[experiment.checkout]`.
- An **arm** is one commit in one experiment, with an exact share of the
  app's subjects, such as `a1b2c3d=0.05`. The commit is the arm's ID.
- The **reference** is the arm that you added first. Evidence compares each
  arm with it. To compare a change with what runs now, add the live commit as
  an arm.
- **Live** gets every request that no experiment takes. Statespace never
  records live.
- Each `start` begins a new **version**. Evidence is per version.
- Settings and experiment definitions are in `statespace.toml`. They change
  only by commits. The file in the build that live runs applies.
- The evidence is on each arm's commit, as the commit status
  `statespace/<app>/<experiment>`, and in `ssp experiment show`.

Statespace never starts an experiment by itself. A person or an agent with
permission on the GitHub repository starts it. Add `--json` to each `ssp`
command to get one JSON document. With `--json`, an error is
`{"error": "..."}` on standard error, with exit status 1. No command asks a
question, except `ssp login`.

## Credentials

The CLI uses the first credential that it finds:

1. `STATESPACE_TOKEN`.
2. The session of `ssp login`.
3. A GitHub user token in `GH_TOKEN` or `GITHUB_TOKEN` (`gho_`, `ghp_`,
   `ghu_`, `github_pat_`).
4. In GitHub Actions, the job's OIDC token. The job needs
   `permissions: id-token: write`.

Check the credential first:

```shell
ssp whoami --json
```

If it fails and you have no GitHub token, ask the user to run `ssp login`.
This command opens a browser, so the user must run it.

## Set up

1. Install the CLI if `ssp` is not available.

   ```shell
   curl -fsSL https://statespace.com/install.sh | bash
   ```

2. Ask the user to install the Statespace GitHub App on the repository from
   https://statespace.com.
3. Write `statespace.toml` in the app's folder. Start from the file in
   [Starter statespace.toml](#starter-statespacetoml). Change the event
   rules and the metric to match the app.
4. Add the step of `statespace-tech/action@v1` after the image build, in the
   workflow that builds every commit that can become an arm. See
   [CI step](#ci-step).
5. Commit `statespace.toml` and the workflow, and push.
6. Add an app for each running copy. In the app's folder, the command reads
   the repository and the folder. Read what it detected.

   ```shell
   ssp app add <app> --note "<what this copy is>" --json
   ```

7. Ask the user to add a runtime token and to run `ssp proxy` in front of the
   app. The token shows only once. Do not write it to files that Git tracks.
   See [Run the proxy](#run-the-proxy).

   ```shell
   ssp token add --app <app> --note "<where the proxy runs>" --json
   ```

8. Make sure that a proxy reports and that the app has a live commit:

   ```shell
   ssp app show <app> --json
   ```

## Run the proxy

The proxy runs in front of live, on the platform of live. Pick the
subcommand from where live runs:

| Live runs on | Subcommand | Each arm runs as |
| --- | --- | --- |
| One machine with Docker | `ssp proxy docker` | A container on the machine |
| Cloud Run | `ssp proxy cloudrun` | A tagged revision with 0% traffic |
| Kubernetes | `ssp proxy kubernetes` | A copy of the Deployment |
| ECS Fargate | `ssp proxy fargate` | An ECS service in the same cluster |
| Fly.io | `ssp proxy fly` | A Machine of the same Fly.io app |

Each subcommand needs `--app` (or `STATESPACE_APP`) and the runtime token
in `STATESPACE_TOKEN`. Units never get a variable that starts with
`STATESPACE_`. The other flags tell where live runs:

```shell
ssp proxy docker     --app shop-dev  --image-file /etc/shop/image
ssp proxy cloudrun   --app shop-prod --service shop --region us-east1
ssp proxy kubernetes --app shop-prod --deployment shop --namespace prod
ssp proxy fargate    --app shop-prod --cluster shop --service shop \
                     --live-url http://internal-shop.example.com
ssp proxy fly        --app shop-prod --fly-app shop
```

The proxy uses the identity of the platform where it runs. Before the
proxy starts, make sure that this identity has these permissions.

### Docker

- The user of the proxy can use the Docker socket.
- `--image-file` holds the live image. A change of the file deploys a new
  live.

### Cloud Run

The proxy runs as its own Cloud Run service. Give its service account:

1. Cloud Run Developer (`roles/run.developer`) on the app's service.
2. Cloud Run Invoker (`roles/run.invoker`) on the app's service.
3. Service Account User (`roles/iam.serviceAccountUser`) on the runtime
   service account of the app's service, so that it can act as it.
4. Artifact Registry Reader (`roles/artifactregistry.reader`) on the
   repository of the images. Cloud Run deploys a revision only when the
   caller can read its image.
5. Logs Viewer (`roles/logging.viewer`) on the project, to read the output
   of the arms.

Gotchas:

- Deploy the proxy service with CPU always allocated and one instance at
  minimum (`--no-cpu-throttling --min-instances 1`). The proxy works
  between requests: it renews its lease and starts and stops arms.
- The template of the app's service keeps the name of live's revision. A
  tool that sends the template back with a new image must clear
  `template.revision`. `gcloud run deploy` does this.

### Kubernetes

The proxy runs as its own Deployment. Its service account needs a Role
that can manage Deployments and Services, and read pods and pod logs, in
the namespace of the app.

### ECS Fargate

The proxy runs as its own ECS service. Its task role needs:

1. The ECS actions `CreateService`, `DeleteService`,
   `DeleteTaskDefinitions`, `DeregisterTaskDefinition`,
   `DescribeServices`, `DescribeTaskDefinition`, `DescribeTasks`,
   `ListServices`, `ListTaskDefinitions`, `ListTasks`,
   `RegisterTaskDefinition`, `TagResource`, and `UpdateService`.
2. `iam:PassRole` on the task role and the execution role of live, for
   `ecs-tasks.amazonaws.com`.
3. `logs:FilterLogEvents` on the log group of live. Live's container must
   log with `awslogs`.

Gotchas:

- Set `--live-url` to live's internal load balancer. Without it, the proxy
  uses live's first Service Connect name.
- The arms use the security group of live. Let the security group of the
  proxy connect to the app's port.
- Use `--unit-ip public` only when the proxy runs outside the VPC, such as
  in a test. It also needs `ec2:DescribeNetworkInterfaces`.

### Fly.io

The proxy runs as its own Fly.io app in the same organization as the app,
such as `shop-statespace` in front of `shop`. Give it:

1. `FLY_API_TOKEN`: a deploy token of the app, from
   `fly tokens create deploy -a shop`. Renew it before it expires.
2. A private Flycast address on the app, from
   `fly ips allocate-v6 --private -a shop`. Live traffic goes to
   `http://shop.flycast`.

Gotchas:

- The app must listen on `[::]`. Arms have no services, so Fly.io sends
  them no public or Flycast traffic. The proxy reaches each arm on its
  private IPv6 address (6PN).
- Fly.io must pull the image of each arm. Push the builds to
  `registry.fly.io/shop` or to a public registry.
- Arms get the secrets of the app. Never set a secret that starts with
  `STATESPACE_` on the app.
- Remove the public addresses of the app, so that all visitors go through
  the proxy.

## Starter statespace.toml

Put this file in the folder of the app's code. It has one experiment, one
shared event, and one metric. Keep the comments.

```toml
# Statespace settings for every app built from this folder.
# A change applies when the build of its commit runs as live.
# Reference: https://docs.statespace.com

# Let testers pick an arm with ?ssp_arm=<experiment>:<commit>.
overrides = false

# Regular expressions. The proxy replaces each match with
# [redacted] before output leaves the machine.
redact = []

# A shared event: it counts in every experiment. Each rule
# uses one source: request, stdout, or stderr.
[[event]]
name = "signup"
match = '''
request.method == "POST"
  && request.path == "/signup"
  && request.status < 300
'''

# A shared metric: a formula over the totals of one arm.
# The first metric is the headline.
[[metric]]
label = "signup rate %"
value = '100.0 * events["signup"].subjects / subjects'

# An experiment. The order of the experiment tables is
# their priority. Set its arms with:
#   ssp experiment update signup --arm <commit>=<share>
[experiment.signup]
# The subject ID. The default is the visitor cookie that
# the proxy sets.
# subject = 'request.headers["x-user-id"]'
# Who may enter. The default is everyone.
# eligibility = 'request.headers["cf-ipcountry"] == "US"'
# The most that the shares of its arms add up to.
max_share = 0.2

# Settings for one app replace the shared settings.
# [app.shop-dev]
# overrides = true
#
# [app.shop-dev.experiment.signup]
# max_share = 1.0
```

- Experiment names have 1 to 40 lowercase letters, digits, and `-`.
- `[[experiment.<name>.event]]` and `[[experiment.<name>.metric]]` add events
  and metrics for one experiment only.
- CI checks the file when it records a build. A wrong key or a CEL error
  fails the step.

## CI step

Add this step after the image build. Give the image by digest. The job needs
`permissions: id-token: write`.

```yaml
- uses: statespace-tech/action@v1
  with:
    image: ghcr.io/acme/shop@${{ steps.build.outputs.digest }}
    path: .          # the folder of statespace.toml
    # app: shop-prod # optional: a build for one app only
```

## Test a change

1. Make the change in a commit. Push it, so that CI builds it and records the
   build.
2. Get the live commit:

   ```shell
   ssp app show <app> --json
   ```

3. Set the arms of the experiment: the live commit first, so that it is the
   reference, then the new commit. A share is a fraction, such as `0.05`.
   Never write `5` or `5%`.

   ```shell
   ssp experiment update <experiment> --app <app> \
     --arm <live commit>=0.05 --arm <commit>=0.05 --json
   ```

   - The command prints what changed and the full list of arms.
   - It fails with "No build of <commit>" when CI did not record the build
     yet. Wait for CI, then try again.
   - `--arm <commit>=0` removes an arm. Arms that you do not name do not
     change.
4. Start the experiment. The command waits until each arm is ready. When an
   arm is not ready in 5 minutes, nothing starts, and the command gives the
   reason for each arm.

   ```shell
   ssp experiment start <experiment> --app <app> \
     --note "<hypothesis>" --json
   ```

5. Read the evidence:

   ```shell
   ssp experiment show <experiment> --app <app> --json
   ```

   - `totals` has `subjects`, `requests`, `errors`, `p50_ms`, and `p90_ms`
     for each arm, by commit.
   - `metrics` has each metric for each arm.
   - `split` tells whether the subjects match the shares. Do not trust
     evidence when `split.ok` is false.
   - `drift.live_commit` tells that live changed since the start.
6. Decide with the user. Statespace gives evidence, not verdicts.
7. Stop the experiment. Statespace keeps its evidence.

   ```shell
   ssp experiment stop <experiment> --app <app> \
     --note "<decision>" --json
   ```

You can leave out `--app` when the current folder has exactly one app.

## Change a running experiment

You cannot change the arms of a running experiment. Do these steps:

1. `ssp experiment stop <experiment>`.
2. `ssp experiment update <experiment> --arm <commit>=<share>`.
3. `ssp experiment start <experiment>`. A new version starts.

## Stop in an emergency

The arms leave traffic in seconds.

```shell
ssp experiment stop <experiment> --app <app> --note "<why>" --json
```

To stop each experiment of an app, get the names, then stop them all:

```shell
ssp experiment list --app <app> --running --json
ssp experiment stop <name> <name> --app <app> --note "<why>"
```

## Events and metrics

```toml
[[event]]
name = "fatal"
match = 'stderr.text.contains("FATAL")'

[[experiment.checkout.event]]
name = "coupon"
match = 'request.path == "/checkout/coupon"'
```

- Each `match` uses one source: `request`, `stdout`, or `stderr`.
- `request` has `method`, `path`, `query`, `headers`, `cookies`, `status`,
  and `ms`.
- `stdout` and `stderr` have `text` and `json` (the line as a JSON object, or
  `{}`).
- A missing field is no match.
- Output events have `.lines` only. Request events have `.lines` and
  `.subjects`.
- A release that changes a shared event, or the event, `subject`,
  `eligibility`, or `max_share` of an experiment, stops the experiment. A
  change of a metric stops nothing.

## Rules

- Do not start an experiment without the user's consent.
- Use `--note` on each command that changes something.
- Never put a token in a file that Git tracks.
- Read the reason when an experiment stops or an arm is not ready.
