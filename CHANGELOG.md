# Changelog

Notable changes to Statespace and its `ssp` CLI. Releases follow semantic versioning.

## 0.5.0 - 2026-11-04

Statespace records counts per subject, not requests, log lines, or bodies. Your own
data tools keep the detail, and each arm carries labels that they can filter by.

### Changed

- **Breaking:** no arm is the reference. `ssp experiment update`, `show`, and `list`
  print no `(reference)`, and commit statuses say `running` on every arm.
- **Breaking:** `ssp proxy` counts. For each request that an arm serves to an
  assigned subject, it adds to that subject's counters in memory: requests,
  `requests_4xx` (status 400 to 499), `requests_5xx` (status 500 to 599), one of 17
  latency buckets, and each event that matches. The buckets have the fixed upper
  bounds 5, 10, 20, 35, 50, 75, 100, 150, 250, 400, 600, 1000, 1500, 2500, 5000, and
  10000 ms, and the last bucket has no bound. It
  sends the changes every 60 seconds and when it stops, to
  `POST /v1/runtime/apps/{app}/counts`, in batches of at most 5,000 subjects with a
  UUID. It sends a failed batch again with the same UUID. It keeps at most 50,000
  subjects in memory and drops the oldest above that, and drops the counts of a
  version that does not run (409). Live and overrides are never counted. A subject
  whose arm is not ready gets live and is counted with no requests, as unavailable.
- **Breaking:** no subject leaves the proxy. It sends
  `hex(HMAC-SHA256(key = app salt, message = subject))`.
- **Breaking:** the proxy sends no rows to `/v1/events`: no runs, requests, stdout,
  stderr, events, or lifecycle rows.
- **Breaking:** `redact` is removed from `statespace.toml`. Statespace stores no
  request data. The backend refuses `redact` as an unknown key.
- **Breaking:** the proxy reads no logs on any runtime: no Docker logs, Kubernetes pod
  logs, CloudWatch Logs, Cloud Logging, or Fly.io logs. Readiness and the reason that
  an arm is not ready come from the platform: the exit code and out of memory on
  Docker, the container states of the pods on Kubernetes (such as `OOMKilled` or
  `ImagePullBackOff`), the stop reason of the tasks on ECS, the conditions of the
  service on Cloud Run, and the state of the Machine on Fly.io. The proxy needs no
  log permission: not Logs Viewer on Cloud Run, not `logs:FilterLogEvents` on Fargate,
  not `pods/log` on Kubernetes. Live's container on Fargate can log with any driver.
- **Breaking:** an event is a CEL rule over one request and its response. A rule sees
  `request` with `method`, `host`, `path`, `query`, `headers`, and `cookies`, and
  `response` with `status`, `headers`, and `ms` (the time to the response headers).
  The proxy evaluates the rules once per arm request, when the response headers
  arrive, and never reads a body. `request.status` and `request.ms` are now
  `response.status` and `response.ms`. Rules over `stdout` and `stderr` are gone.
- **Breaking:** `[[metric]]` is gone. The results give, for each arm, latency p50,
  p95, and p99 in whole ms, and events. The events are `requests_4xx` and
  `requests_5xx` (the share of subjects with at least one response of that class),
  then each `[[event]]` in file order (the share of subjects with at least one
  match). Each share is a fraction with 4 decimals and its 95% Wilson interval.
  Below 30 subjects in an arm, there is no interval. Commit statuses say, for
  example, `signup 0.0261 [0.0184, 0.0368] · 1,188 subjects`, or `requests_5xx …`
  without events.
- **Breaking:** event names are snake_case (`^[a-z][a-z0-9_]*$`), and
  `requests_4xx` and `requests_5xx` are reserved. The proxy leaves out an event
  with another name, and says why.
- **Breaking:** the proxy sends `ssp-experiment` and `ssp-arm` to an arm, in place of
  `x-statespace-experiment` and `x-statespace-arm`. It removes these headers when a
  client sends them. It sends no `x-statespace-subject`.
- Each arm carries labels for your own data tools. Its environment has `SSP_EXPERIMENT`,
  `SSP_ARM` (the full commit), and `OTEL_RESOURCE_ATTRIBUTES` with live's attributes
  and `statespace.experiment`, `statespace.arm`, and `service.version` of the arm. A
  subject that an arm serves gets the cookie `ssp_arms=<experiment>:<commit>`, which
  JavaScript can read (`Path=/`, `SameSite=Lax`, 90 days). A subject in no arm loses
  it. This cookie works on `ssp proxy s3` and `ssp proxy gcs` too.
- **Breaking:** on Docker, the labels of an arm's container are `com.statespace.app`,
  `com.statespace.unit`, `com.statespace.experiment`, `com.statespace.commit`,
  `com.statespace.image`, and `com.statespace.proxy`, in place of
  `statespace.com/*`. On Fly.io, the metadata of an arm's Machine is
  `statespace_app`, `statespace_experiment`, `statespace_commit`, `statespace_image`,
  and `statespace_port`. A proxy of this version does not adopt the arms of an
  earlier version: stop the experiments before you upgrade.
- **Breaking:** `ssp experiment show` prints the results of a version, where to find
  the arms in your own data tools, the override links while the version runs, and
  the versions:

  ```text
  checkout v1 · shop-prod · since 2026-11-04 09:12 UTC · 2,392 subjects · split ok

  latency (ms)                 c0ffee1                   a1b2c3d
    p50                        41                        44
    p95                        180                       205
    p99                        420                       610

  events (estimate [95% CI])   c0ffee1                   a1b2c3d
    requests_4xx               0.1121 [0.0950, 0.1318]   0.1178 [0.1003, 0.1378]
    requests_5xx               0.0274 [0.0193, 0.0388]   0.0438 [0.0331, 0.0578]
    signup                     0.0208 [0.0136, 0.0306]   0.0261 [0.0184, 0.0368]
    search                     0.2804 [0.2554, 0.3068]   0.2753 [0.2499, 0.3007]

  Find it in your own data tools: statespace.arm = "c0ffee1…" | "a1b2c3d…"

  Override links:
    c0ffee1  ?ssp=checkout:c0ffee1
    a1b2c3d  ?ssp=checkout:a1b2c3d
  ```

  The first line gives the version, the app, its time window (`<start> – <stop> UTC`
  when it stopped), the subjects of all arms, and the split check (`ok`, `off`, or
  `–` below 100 subjects). Below 30 subjects, an event is `0.0261 [–]`. Before the
  first start, `show` prints `<experiment> · <app> · not started`. `--version N`
  shows the results of version N; a stopped version shows the results of its end.
  For the arms, use `ssp experiment update <experiment>` without `--arm`.
- **Breaking:** `--json` carries exactly the information of the text output,
  structured, and never more. `ssp experiment show --json` prints `experiment`,
  `version`, `app`, `started_at`, `stopped_at`, `subjects`, `split {ok}`, `arms`,
  `lookup {"statespace.arm": [...]}`, `override_links` (`{"<commit>": "<link>"}`
  while the version runs, else null), and `versions`. Each arm has `commit`,
  `subjects`, `latency_ms {p50, p95, p99}`, and `events`, in order, each
  `{"estimate", "ci95": [low, high]}`. `ci95` is null below 30 subjects.
  The other commands follow the same rule: `app list`, `app show`, `app add`,
  `app remove`, `experiment list`, `experiment update`, `experiment start`,
  `experiment stop`, `token add`, `token list`, `token remove`, `login`, and
  `whoami` print only the fields of their text, not the document of the API.

### Removed

- **Breaking:** `ssp db url` and `ssp db rotate`. There is no SQL access. Read the
  results with `ssp experiment show`, its `--json`, or the dashboard.

### Fixed

- While the proxy cannot read what live runs, such as while a new live starts on
  ECS Fargate, it reports the last live that it read. Before, its reports named no
  live, and the experiments of the app stopped until the new live was ready.

## 0.4.3 - 2026-11-03

`ssp` runs alongside live, never inside it. It never changes a resource that it did
not create, and each arm is a resource that it creates, labels, and deletes.

### Changed

- **Breaking:** `ssp proxy` takes no `--app` and reads no `STATESPACE_APP`. The proxy
  asks Statespace which app its runtime token is for (`GET /v1/runtime/app`). When
  Statespace refuses the token, the proxy stops with an error. `--app` stays on the
  other commands, such as `ssp experiment` and `ssp token`.
- The proxy starts in two phases. It reaches live from its flags alone, listens, and
  sends every request to live at once. In the background, it asks Statespace for its
  app until Statespace answers. Then it reports, reads the configuration, and routes
  experiments. So a proxy that starts or restarts while Statespace is down still
  serves the app.
- **Breaking:** `ssp proxy cloudrun` runs each arm as its own Cloud Run service,
  `<service>-ssp-<experiment>-<commit 12>`, in the project and region of live. A name
  longer than 49 characters ends with a short hash. The service is a copy of live's
  template with only the image changed, with live's ingress, launch stage, Binary
  Authorization policy, and labels, and the labels `statespace-app`,
  `statespace-experiment`, and `statespace-commit`. It gets no traffic split, custom
  domain, or IAM policy of live. Only the proxy's service account may call it. The
  proxy never changes live's service: it adds no revision, tag, or label to it, and
  does not pin its traffic. A deploy with any tool, also a full apply with Terraform,
  works as before. Stop deletes the arm's service.
- **Breaking:** the proxy on Cloud Run needs other permissions. On the project:
  `run.services.create`, `run.services.delete`, `run.services.get`,
  `run.services.list`, `run.services.getIamPolicy`, `run.services.setIamPolicy`,
  `run.revisions.get`, `run.operations.get`, and `run.routes.invoke`, for example in
  a custom role. It also needs Service Account User on live's runtime service
  account, Artifact Registry Reader on the image repository, and Logs Viewer. It no
  longer needs Cloud Run Developer on live's service. `examples/cloudrun` shows the
  commands.
- **Breaking:** `ssp proxy docker --live-container <name>` forwards live traffic to a
  container that you run, such as the container of a Compose service. The proxy does
  not run live. When you create the container again, such as with a new image, the
  proxy sends live traffic to the new container within seconds, and reports its
  image as live. `--live-port` sets live's port when the container does not expose
  exactly one TCP port.
- **Breaking:** on Docker, each arm is a copy of live's container with another
  image: live's own environment (not what live's image sets), command and entry
  point when live sets them, mounts, limits, and networks. An arm gets none of live's
  labels, names, network aliases, published ports, or restart policy, and no
  variable that starts with `STATESPACE_`. Thus `docker compose up --remove-orphans`
  and `docker compose down` do not count the arms as Compose's own.
- **Breaking:** `--image-file`, `--port`, `--volume`, `--memory`, and `--cpus` of
  `ssp proxy docker` are removed. Arms no longer get the proxy's environment.
- `ssp proxy docker` talks to the Docker Engine API over the socket of
  `DOCKER_HOST` (`unix://` or `tcp://` without TLS), or `/var/run/docker.sock`. It
  needs no `docker` command, so it runs in the `ssp` image with the socket mounted.
  It pulls arm images with the credentials in `auths` of the Docker configuration;
  it does not read credential helpers.
- On Docker, when the proxy runs in a container, it reaches live and the arms at
  their addresses on a network that it shares with live. On the host, it reaches
  live at the port that live publishes, and the arms publish their port on
  127.0.0.1.
- When the Docker proxy stops, it gives each arm's container one second to stop,
  not ten, so that it removes them before Compose or systemd ends it.
- `examples/fastapi` runs the shop and the proxy with Docker Compose
  (`compose.yaml`), not with systemd.

## 0.4.2 - 2026-11-03

### Added

- The release workflow signs the image `ghcr.io/statespace-tech/statespace` with
  cosign, without a key, by its digest. The image has an SBOM, a build provenance,
  and OCI labels: source, version, revision, licenses, title, and description. Each
  archive of the GitHub release has a build provenance. `SECURITY.md` tells how to
  verify them.

### Changed

- The image is on `gcr.io/distroless/static-debian12:nonroot`, pinned by digest. It
  has no shell and no package manager. It holds `ssp`, `statespace` as a hard link to
  `ssp`, and the CA certificates, and it runs as the user `nonroot` (65532). A command
  that starts a shell in the image, such as `docker run --entrypoint sh`, does not
  work.
- The release build uses full link-time optimization and one codegen unit. The `ssp`
  binary for Linux x86_64 is 13.9 MB, not 21.0 MB. The compressed image for
  linux/amd64 is 6.9 MB, not 12.3 MB.
- CI checks the advisories, licenses, and sources of the dependencies with
  cargo-deny. Dependabot updates the actions, the base images, and the crates each
  week.

### Fixed

- The proxy cuts the reason of a unit in a report to 1500 bytes, not 1500
  characters, and sends no reason for an empty one. Before, Statespace refused the
  whole report when a long reason had characters that are not ASCII.
- The proxy cuts the host name in its instance name to 120 bytes. Before, Statespace
  refused the reports of a proxy on a host with a very long name.
- The proxy sends no address for a unit that has none yet, such as a Fargate task
  that is still starting. Before, Statespace refused the whole report while the
  task started.

## 0.4.1 - 2026-11-03

### Added

- `ssp proxy s3 --bucket <bucket>` and `ssp proxy gcs --bucket <bucket>` serve a static
  site from a bucket. Live is the bucket root. Each arm is a build folder, such as
  `s3://acme-site/builds/<commit>/`, that CI records with `files` in the action. The
  proxy serves the files like a static website endpoint: `index.html` for a folder,
  then `<path>/index.html` and `<path>.html`, and the build's `404.html` with status
  404. It never writes to the bucket.
- A build folder is ready only while its files hash is the hash that CI recorded. The
  proxy checks each folder again every 60 seconds. A changed or missing build is not
  ready, its subjects go to live, and `ssp experiment show` gives the reason. The
  proxy reports live by the files hash of the bucket root, without the folders that
  hold builds.
- `--fallback <file>` serves that file of the build, with status 200, for a path
  without a file extension that matches no file. Use it for a single-page app.
- `--cache` keeps files in memory, in one cache of 256 MB: the files of an arm until
  the arm stops, and the files of live for 10 seconds.
- `ssp proxy s3` uses the AWS default credential chain. `--endpoint` serves an
  S3-compatible store, such as Cloudflare R2, Backblaze B2, DigitalOcean Spaces, or
  MinIO. `ssp proxy gcs` uses the metadata server or the Application Default
  Credentials: a user, a service account key, or workload identity federation.

### Changed

- Image runtimes ignore files builds, and file runtimes ignore image builds. An arm
  whose build the runtime cannot run is not ready, and the report says why.
- **Breaking:** overrides are always on. The `overrides` setting is removed from
  `statespace.toml`.
- **Breaking:** the override parameter, cookie, and header are all `ssp`:
  `?ssp=<experiment>:<commit>`, the cookie `ssp`, and the header `ssp: <value>`.
  `ssp_arm` and `ssp-arm` are removed.
- A command for a workspace that Statespace has not activated yet prints one
  line, `<workspace> is pending: Statespace is reviewing your access. See
  https://statespace.com/contact`, and exits 1. With `--json`, the line is
  the `error`.

### Fixed

- A response of an arm without a `Cache-Control` rule of its own gets
  `private, no-cache` and no `Last-Modified`. A browser asks again before it
  uses the response again, so a changed arm or a new override shows at once.

## 0.4.0 - 2026-11-02

Experiments with commit arms. This release replaces the `exp/` branch model. Nothing of
the old model stays.

### Added

- `statespace` is a second name for `ssp`. `cargo install` installs both, and
  `install.sh` and the image add `statespace` as a link to `ssp`.
- `ssp app add | remove | list | show <name>`. `ssp app add` reads the repository from
  the Git remote `origin` and the folder from the nearest `statespace.toml`, or takes
  `--repo` and `--path`, and prints what it detected.
- `ssp experiment list | show | update | start | stop`. An experiment is an
  `[experiment.<name>]` table in `statespace.toml`, with `subject`, `eligibility`,
  `max_traffic`, and its own events and metrics.
- `ssp experiment update <name> --arm <commit>=<share>` changes only the arms that it
  names. `=0` removes an arm. Shares are fractions: `5` and `5%` fail. It prints the
  change, the full list with the reference, and the total of `max_traffic`.
- `ssp experiment start` begins a new version, waits until each arm is ready, and prints
  each arm, or the reason per arm when one is not ready. It tells which running
  experiments can claim the same requests.
- `ssp experiment stop` takes one or more names.
- `ssp experiment list --running` and `--all`. `ssp experiment show` prints the
  definition, the arms, the evidence per arm, the versions, the history, the drift from
  live, and the override links.
- `--app` is optional when the current folder is linked to exactly one app.
- `ssp token add | remove | list` for runtime tokens, one app each.
- `--note` (1 to 200 characters, one line) on every command that changes something.
- Credentials: `STATESPACE_TOKEN`, then the `ssp login` session, then a GitHub user
  token in `GH_TOKEN` or `GITHUB_TOKEN`, then the OIDC token of a GitHub Actions job.
  `ssp whoami` shows which one the CLI uses.
- `ssp proxy docker`, `ssp proxy cloudrun`, `ssp proxy kubernetes`,
  `ssp proxy fargate`, and `ssp proxy fly`. One
  `Orchestrator` trait runs the units: one for each arm of each starting or running
  experiment. Every instance routes; the instance that holds the app's lease
  orchestrates.
- The proxy checks the running experiments in the order of `statespace.toml`. Each
  experiment reads its own subject. The first experiment whose arm range holds the
  subject's point takes the request. Else live serves it.
- Cloud Run: units run as revisions with 0% traffic and the tag
  `ssp-<experiment>-<commit 12>`. The proxy pins live's traffic only while it makes
  the revision of a unit, then gives the service its own template and traffic back
  and adds only the unit's tag, so a deploy of a new live gets all the traffic. It
  sends an ID token for the service's URL, and reads the output of all units with
  one Cloud Logging query every 5 seconds. When Cloud Logging answers 429, the proxy
  waits and loses no line.
- Kubernetes: the proxy runs as its own Deployment. Units run as copies of the app's
  Deployment, `<deployment>-ssp-<experiment>-<commit 12>`, each with a Service.
- ECS Fargate (`ssp proxy fargate`): each unit is an ECS service in live's cluster,
  from a copy of live's task definition with another image. The proxy sends to the
  tasks' IP addresses, reads output from CloudWatch Logs, and uses the AWS default
  chain for credentials. `examples/fargate` has the proxy's task definition,
  service, and policy.
- Fly.io (`ssp proxy fly --fly-app <app>`): each unit is a copy of a live Machine
  with the unit's image, without services, mounts, or `fly_*` metadata, so it gets
  no public traffic and `fly deploy` leaves it alone. The proxy reaches units on
  the private network.
- Unit names that pass a platform limit end with a short hash. Labels keep the full
  experiment and commit.
- `[[event]]` rules from `statespace.toml`, checked in the proxy on requests, standard
  output, and standard error. A unit gets the shared events and the events of its
  experiment. Event names are lowercase snake_case, such as `purchase` or
  `add_to_cart`: a lowercase letter, then lowercase letters, digits, or `_`, at most
  64 characters. CI refuses other names.
- The proxy records `runs`, `requests`, `stdout`, `stderr`, `events`, and `lifecycle`
  rows for units. Each row has the experiment, the arm, and the version. Each event
  row names the request or output line that matched it.
- `ssp db url` prints the connection string of your own read-only PostgreSQL
  login, so `psql "$(ssp db url)"` works. The login reads 8 views of the apps
  whose repository you can read, with no schema name: `experiments`,
  `versions`, `arms`, `builds`, `assignments`, `requests`, `logs`, and
  `events`. `events.request_id` and `events.log_id` join to `requests` and
  `logs`. The password stays the same until `ssp db rotate --note <why>`,
  which stops the old password and ends its sessions. `--account` names the
  workspace. The connection string has `sslmode=require`, so it works with
  every PostgreSQL client. `--verify` returns
  `sslmode=verify-full&sslrootcert=system` instead, which checks the server's
  certificate with the system trust store (libpq 16 and later).
- The CLI names the root folder of a repository "the root", as the action
  does. Commands in the root folder find the app that is linked to it.
- `ssp experiment show` shows the note of each stopped version, in the column
  `NOTE`. When Statespace stops a version, the note says why, such as
  `definition changed in live d00d123`. `--json` has `versions[].stop_note`.
- The starter `statespace.toml` and the CI step are in `skills/statespace/SKILL.md`.

### Changed

- All settings come from `statespace.toml` through the runtime configuration. The proxy
  has no flags for settings.
- Overrides name the experiment and the commit: `?ssp_arm=checkout:a1b2c3d` or
  `ssp-arm: checkout:a1b2c3d`. Override visits count for the experiment.
- The visitor cookie lasts 90 days, renews on every response, and works in iframes:
  `SameSite=None; Secure; Partitioned` over `https`. The proxy sets it only while a
  running experiment uses the default subject.
- The proxy sends `x-statespace-subject` and, for an arm, `x-statespace-experiment` and
  `x-statespace-arm` with the full commit.
- Environment variables: `STATESPACE_TOKEN` replaces `SSP_TOKEN`, and
  `STATESPACE_API` replaces `STATESPACE_URL`.
- The example app listens on IPv6 and IPv4.
- Pushes run only the fast tests. The runtime tests (Docker, Kubernetes, and real
  Cloud Run, ECS Fargate, and Fly.io) run before each release, and an admin can run
  them on `main`.

### Removed

- `ssp apply`, `ssp status`, `ssp stop`, `ssp resume`, and their targets: pull
  request and repository URLs, paths, and `exp/` branches.
- Holds: `ssp app stop` and `resume`, `ssp arm stop` and `resume`.
- `ssp arm`. `ssp experiment update` sets the arms.
- `ssp init`. A person or a coding agent writes `statespace.toml` from the starter.
- The shared control, the `live` keyword, and the header `x-statespace-side`.
- The app-wide settings `max_traffic`, `max_arms`, `arm_role`, `subject`, and
  `eligibility`. Each experiment has its own `max_traffic`, `subject`, and
  `eligibility`.
- `ssp controller kubernetes`. `ssp proxy kubernetes` replaces it.
- `ssp token create` and `revoke`, presets, and scopes.
- The proxy flags `--overrides`, `--redact`, `--max-arms`, `--arm-env`,
  `--stale-after`, `--image`, and `--default`, and the global `--workspace`.

## 0.3.0 - 2026-10-24

### Added

- `ssp proxy --overrides` lets a request name the running arm that serves it. A browser
  opens a URL with `?ssp_arm=exp/price/lower`; a cookie keeps the arm for an hour, renewed
  on each request, and `?ssp_arm=` ends it. An API client sends the header
  `ssp-arm: exp/price/lower`. A branch that does not run gets a page that lists the
  running arms. Statespace records these visits as runs with the reason `override`.
- `ssp stop --reason <text>`. The check, `ssp status`, and the dashboard show it.
- Targets name an app (`sandbox`), an experiment (`sandbox/price`), or an arm by its
  branch (`exp/price/lower`).

### Changed

- `ssp stop` places a hold in Statespace instead of making pull requests drafts. Pull
  requests do not change, and a stop does not need GitHub. A hold covers the arms that
  start later too. `ssp resume` removes exactly the hold on each target.
- `ssp status` marks the arms that a hold stops and lists every hold with who placed it,
  when, and why. Its JSON is `{"arms": [...], "holds": [...]}`.
- The proxy removes `?ssp_arm=`, the `ssp-arm` header, and the `ssp_arm` cookie before the
  app gets a request, also without `--overrides`.
- Responses that an arm serves are `Cache-Control: private`.

### Security

- Dependencies with RustSec advisories are updated: rustls, rustls-webpki,
  webpki-roots, and tokio.

## 0.2.0 - 2026-10-22

Experiments run from pull requests. A branch named `exp/<name>` is an experiment with one
arm, and `exp/<experiment>/<arm>` is one arm of a larger experiment. An arm runs while
GitHub says that its pull request can merge. This release removes the terminal flow of
0.1.0.

### Added

- The Statespace GitHub App opens a "Set up Statespace" pull request with
  `statespace.toml` and a workflow that builds the `Dockerfile` nearest the root and
  records each commit of an `exp/` branch. The file sets `max_traffic = 0.2`.
- One `Statespace` check on each commit says what the arm runs, or why it waits, with the
  experiment's total share. It fails only when CI could not record the commit. While a
  version runs, it shows the evidence of each arm and control: subjects, requests,
  errors, p90 latency, events, and whether the split matches the shares.
- A workspace is the GitHub organization or user that installed the App. People join
  every workspace whose installation they can access, and act as their role on each
  repository allows. `--workspace` and `STATESPACE_WORKSPACE` choose a workspace;
  `status`, `stop`, and `resume` use the owner of their target's repository.
- `statespace.toml` gives the app, an arm's share, subject rule, and eligibility rule, and
  the app's `max_traffic` and `output_retention_days`. App settings take effect from the
  default branch.
- `ssp apply` records a CI job's image with the job's GitHub OIDC token. On the default
  branch, the image becomes a default build; on an `exp/` branch, a version of the arm.
  `--dry-run` checks `statespace.toml` and prints the plan anywhere.
- `ssp status` shows what runs, and why, and each experiment's share of the app's
  subjects. Without a target, it shows the arm of the `exp/` branch you are on, else the
  app or repository you are in.
- `ssp stop` makes pull requests drafts as the Statespace App, so their arms stop at once.
  `ssp resume` marks ready only the drafts that a stop made. Both take pull request URLs,
  repository URLs, and paths, and exit 1 when they cannot change a pull request.
- Each base commit of an experiment's arms has its own control.

### Changed

- `ssp proxy` runs containers with `--image` or `--image-file`, and only routes with
  `--default`. It reports the digest of its default.
- `ssp controller` is `ssp controller kubernetes`.
- `--max-variants` is `--max-arms`, and `--variant-env` is `--arm-env`.
- Events, views, headers, and container variables name an `arm` and its `commit`:
  `x-statespace-arm`, `STATESPACE_ARM`, and `STATESPACE_COMMIT`.
- `ssp login` signs in with GitHub only.
- The `runtime` preset also has `arms:report`, for `ssp proxy` and `ssp controller`. A
  runtime token can be for one app; then the proxy and the controller take the app from
  it, and `--app` is optional.
- The dashboard erases a subject and deletes an app.

### Removed

- `ssp app`, `ssp variant`, `ssp experiment`, `ssp keygen`, `--runtime`, `--trust-key`,
  signing keys, and the `publisher` and `controller` token presets.

## 0.1.0 - 2026-10-15

The first release. Statespace compares whole versions of an app in production. A variant is
a signed image; each experiment compares it with a control arm of the deployed image.

### Added

- `ssp app` creates apps. Each app has its own variants, experiments, and schema of
  analysis views.
- `ssp variant create --image` resolves an image's tag to its digest with `docker` and
  signs the digest, app, name, and commit with a key from `ssp keygen`.
- `ssp experiment` takes `--split ARM=SHARE`, each arm's share of all the app's subjects.
  Control gets the largest variant's share unless `control=SHARE` sets it. Running
  experiments of an app hold disjoint shares of its subjects.
- `ssp proxy`, a reverse proxy that assigns each request's subject and forwards it to the
  app's default, an experiment's control arm, or a variant. With `--runtime docker`, it
  runs the default and every arm as containers, adopts them after a crash, and swaps the
  default on SIGHUP without dropping requests. With `--runtime kubernetes`, it runs beside
  the default in each pod.
- `ssp controller` runs each arm on Kubernetes as a copy of the default's Deployment with
  the arm's image, reports where it runs, and records the lines its pods print.
- Arms record every line they print, every request, lifecycle events, and per-minute
  metrics. The default is never recorded.
- `ssp token` creates, lists, and revokes tokens and sessions. One token opens the API and,
  with `query:read`, PostgreSQL. Presets: `runtime`, `controller`, `publisher`, `analyst`,
  and `admin`.
- `ssp whoami`, `ssp logout --all`, and `ssp keygen`.
- `ssp app erase-subject` erases a subject's data in an app.
- The `ghcr.io/statespace-tech/statespace` image, for the proxy sidecar and the controller.
- `examples/fastapi`, a FastAPI shop with a CI workflow and a systemd unit, and
  `examples/kubernetes`, its manifests for Kubernetes.
- Commands print text. `--json` prints one JSON document instead.
- An agent skill at `statespace.com/skill.md`.
