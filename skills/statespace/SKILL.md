---
name: statespace
description: Run commits of an app as arms of an experiment on shares of live traffic with Statespace, and read the evidence. Use when asked to set up Statespace, test a code change on live traffic, compare a commit with production, or read experiment results.
---

# Statespace

Statespace runs commits of an app on small shares of live traffic and counts
what happens to their subjects.

- An **app** is one running copy of a service, such as `shop-prod`. A proxy is
  in front of it.
- An **experiment** compares commits in one app. Its table in
  `statespace.toml` defines it, such as `[experiment.checkout]`.
- An **arm** is one commit in one experiment, with an exact share of the
  app's subjects, such as `a1b2c3d=0.05`. The commit is the arm's ID.
- No arm is special. To compare a change with what runs now, add the live
  commit as an arm.
- **Live** gets every request that no experiment takes. Statespace never
  counts live or overrides.
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

A new workspace starts as pending. Until Statespace staff activate it, every
command fails with `<workspace> is pending: Statespace is reviewing your
access`. Tell the user, and stop. Retrying does not help.

## Set up

1. Install the CLI if `ssp` is not available.

   ```shell
   curl -fsSL https://statespace.com/install.sh | bash
   ```

2. Ask the user to install the Statespace GitHub App on the repository from
   https://statespace.com.
3. Write `statespace.toml` in the app's folder. Start from the file in
   [Starter statespace.toml](#starter-statespacetoml). Change the event
   rules to match the app.
4. Add the step of `statespace-tech/action@v1` after the build, in the
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
| One machine with Docker | `ssp proxy docker` | A copy of live's container |
| Cloud Run | `ssp proxy cloudrun` | A copy of live's service |
| Kubernetes | `ssp proxy kubernetes` | A copy of the Deployment |
| ECS Fargate | `ssp proxy fargate` | An ECS service in the same cluster |
| Fly.io | `ssp proxy fly` | A Machine of the same Fly.io app |
| S3 (static site) | `ssp proxy s3` | A build folder in the bucket |
| Cloud Storage (static site) | `ssp proxy gcs` | A build folder in the bucket |

Each subcommand needs the runtime token in `STATESPACE_TOKEN`. The proxy
asks Statespace which app the token is for, so it takes no `--app`. It
sends every request to live from the start, also while Statespace is
down, and routes experiments once Statespace names the app. Units
never get a variable that starts with `STATESPACE_`. The proxy never
changes live: you run live as before, and each arm is a resource that the
proxy makes, labels, and deletes. The flags tell where live runs:

```shell
ssp proxy docker     --live-container shop
ssp proxy cloudrun   --service shop --region us-east1
ssp proxy kubernetes --deployment shop --namespace prod
ssp proxy fargate    --cluster shop --service shop \
                     --live-url http://internal-shop.example.com
ssp proxy fly        --fly-app shop
ssp proxy s3         --bucket acme-site
ssp proxy gcs        --bucket acme-site
```

The proxy uses the identity of the platform where it runs. Before the
proxy starts, make sure that this identity has these permissions.

### Docker

- The proxy can use the Docker socket. In a container, mount
  `/var/run/docker.sock` and run as root.
- `--live-container` names the container that runs live, such as the
  container of a Compose service. The proxy forwards to it and follows it
  when it is created again. `--live-port` sets its port when it exposes
  more than one.
- When the proxy runs in a container, it must share a Docker network with
  live. Compose puts the services of a project on one network. On the
  host, live must publish its port.
- Each arm is a copy of live's container with another image: live's own
  environment, networks, mounts, and limits. It has no Compose labels, so
  Compose ignores it.

### Cloud Run

The proxy runs as its own Cloud Run service. Each arm is its own Cloud Run
service, `<service>-ssp-<experiment>-<commit 12>`, a copy of live's service
with another image. The proxy never changes live's service. Give the
proxy's service account:

1. A custom role on the project with `run.services.create`,
   `run.services.delete`, `run.services.get`, `run.services.list`,
   `run.services.getIamPolicy`, `run.services.setIamPolicy`,
   `run.revisions.get`, `run.operations.get`, and `run.routes.invoke`.
2. Service Account User (`roles/iam.serviceAccountUser`) on the runtime
   service account of the app's service, so that the arms can run as it.
3. Artifact Registry Reader (`roles/artifactregistry.reader`) on the
   repository of the images. Cloud Run deploys an image only when the
   caller can read it.

Gotchas:

- Deploy the proxy service with CPU always allocated and one instance at
  minimum (`--no-cpu-throttling --min-instances 1`). The proxy works
  between requests: it renews its lease and starts and stops arms.
- An arm's service copies the ingress of live's service. The proxy must
  reach both.

### Kubernetes

The proxy runs as its own Deployment. Its service account needs a Role
that can manage Deployments and Services, and read pods, in the namespace
of the app.

### ECS Fargate

The proxy runs as its own ECS service. Its task role needs:

1. The ECS actions `CreateService`, `DeleteService`,
   `DeleteTaskDefinitions`, `DeregisterTaskDefinition`,
   `DescribeServices`, `DescribeTaskDefinition`, `DescribeTasks`,
   `ListServices`, `ListTaskDefinitions`, `ListTasks`,
   `RegisterTaskDefinition`, `TagResource`, and `UpdateService`.
2. `iam:PassRole` on the task role and the execution role of live, for
   `ecs-tasks.amazonaws.com`.

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
- Remove the public addresses of the app, so that all requests go through
  the proxy.

### S3 and Cloud Storage

For a static site. Live is the bucket root. Each build is a folder of
files in a bucket, such as `s3://acme-site/builds/<commit>/`. The proxy
serves the files itself, like a static website endpoint, and never writes
to the bucket.

- `ssp proxy s3` uses the AWS default credential chain. Give it
  `s3:GetObject` and `s3:ListBucket` on the bucket. `--endpoint` serves an
  S3-compatible store, such as Cloudflare R2 or MinIO.
- `ssp proxy gcs` uses the metadata server or the application default
  credentials. Give it Storage Object Viewer
  (`roles/storage.objectViewer`) on the bucket.

Gotchas:

- Each deploy of live must run the action too. The proxy finds live by the
  files hash of the bucket root, without the folders that hold builds.
- Exclude the build folders from deploys that delete files, such as
  `aws s3 sync dist/ s3://acme-site/ --delete --exclude 'builds/*'`.
- A build must not change after CI records it. A changed or missing build
  is not ready, and its subjects go to live.
- `--fallback 200.html` serves that file of the build for a path without a
  file extension that matches no file. Use it for a single-page app.
- `--cache` keeps files in memory, in one cache of 256 MB.
- A CDN in front of the proxy must not cache pages per subject. Arm
  responses get `Cache-Control: private, no-cache`.

## Starter statespace.toml

Put this file in the folder of the app's code. It has one experiment and one
shared event. Keep the comments.

```toml
# Statespace settings for every app built from this folder.
# A change applies when the build of its commit runs as live.
# Reference: https://docs.statespace.com

# A shared event: it counts in every experiment. Each rule
# sees one request and its response. The first event is
# the headline.
[[event]]
name = "signup"
match = '''
request.method == "POST"
  && request.path == "/signup"
  && response.status < 300
'''

# An experiment. The order of the experiment tables is
# their priority. Set its arms with:
#   ssp experiment update signup --arm <commit>=<share>
[experiment.signup]
# The subject ID. The default is the cookie ssp_sid that
# the proxy sets.
# subject = 'request.headers["x-user-id"]'
# Who may enter. The default is everyone.
# eligibility = 'request.headers["cf-ipcountry"] == "US"'
# The most that the shares of its arms add up to.
max_traffic = 0.2

# Settings for one app replace the shared settings.
# [app.shop-dev.experiment.signup]
# max_traffic = 1.0
```

- Experiment names have 1 to 40 lowercase letters, digits, and `-`.
- Event names are lowercase snake_case: a lowercase letter, then lowercase
  letters, digits, or `_`, at most 64 characters, such as `add_to_cart`.
- `[[experiment.<name>.event]]` adds events for one experiment only.
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

For a static site on `ssp proxy s3` or `ssp proxy gcs`, upload the files of
each commit to a folder, then give the folder as `files`. The location ends
with `/`. The job needs read access to the folder.

```yaml
- run: aws s3 sync dist/ s3://acme-site/builds/${{ github.sha }}/
- uses: statespace-tech/action@v1
  with:
    files: s3://acme-site/builds/${{ github.sha }}/
    path: .
```

## Test a change

1. Make the change in a commit. Push it, so that CI builds it and records the
   build.
2. Get the live commit:

   ```shell
   ssp app show <app> --json
   ```

3. Set the arms of the experiment: the live commit and the new commit. A
   share is a fraction, such as `0.05`.
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

5. Read the results:

   ```shell
   ssp experiment show <experiment> --app <app> --json
   ```

   - `arms` has, for each arm:
     - `subjects`: the subjects that the arm served.
     - `latency_ms`: `p50`, `p95`, and `p99`, in whole ms.
     - `events`: `requests_4xx` and `requests_5xx` (the share of subjects
       with at least one response of 400 to 499, or 500 to 599), then
       each event of `statespace.toml` in file order (the share of
       subjects with at least one match).
   - An event has an `estimate` and its 95% interval `ci95`
     (`[low, high]`). Both are fractions. `ci95` is null below 30
     subjects.
   - `split.ok` tells whether the subjects match the shares. Do not trust
     the results when it is false.
   - `lookup` gives the label of the arms, to find the version in the
     user's own tools.
   - `--version N` shows the results of an earlier version.
   - The text output gives the same information as `--json`.
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

## Events

```toml
[[event]]
name = "account"
match = 'response.headers["x-statespace-event"] == "account_created"'

[[experiment.checkout.event]]
name = "coupon"
match = 'request.path == "/checkout/coupon" && response.status == 200'
```

- Each rule sees one request and its response, when the response headers
  arrive. The proxy never reads a body.
- `request` has `method`, `host`, `path`, `query`, `headers`, and
  `cookies`.
- `response` has `status`, `headers`, and `ms` (the time to the response
  headers).
- `subject` and `eligibility` use `request` only.
- A missing field is no match.
- To count an event that no request shows, let the app name it in a
  response header, and match that header.
- An event name is snake_case, such as `add_to_cart`. `requests_4xx`
  and `requests_5xx` are reserved: the results always show them.
- The results give, for each event, the share of subjects with at least
  one match.
- A release that changes a shared event, or the event, `subject`,
  `eligibility`, or `max_traffic` of an experiment, stops the experiment.

## Find an arm in your own tools

The proxy labels each arm. Live gets none of these labels.

- The arm's resources: labels or tags with the experiment and the commit.
- The arm's environment: `SSP_EXPERIMENT`, `SSP_ARM`, and
  `OTEL_RESOURCE_ATTRIBUTES` with `statespace.experiment`, `statespace.arm`,
  and `service.version`.
- Each request to an arm: the headers `ssp-experiment` and `ssp-arm`.
- The subject's browser: the cookie `ssp_arms`, such as
  `checkout:<commit>`, which JavaScript can read.

`ssp experiment show` prints the label to search for, under "Find it in
your own tools": `statespace.arm`.

## Rules

- Do not start an experiment without the user's consent.
- Use `--note` on each command that changes something.
- Never put a token in a file that Git tracks.
- Read the note when an experiment stops, and the reason when an arm is not
  ready.
