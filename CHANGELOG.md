# Changelog

Notable changes to Statespace and its `ssp` CLI. Releases follow semantic versioning.

## Unreleased

### Added

- `ssp proxy fargate` runs the arms of experiments on ECS Fargate. Each unit
  is an ECS service in live's cluster, from a copy of live's task definition
  with another image. The proxy sends to the tasks' IP addresses, reads
  output from CloudWatch Logs, and uses the AWS default chain for
  credentials. `examples/fargate` has the proxy's task definition, service,
  and policy.
- The runtime tests also run `ssp proxy fargate` on real ECS Fargate in a
  sandbox account of AWS.
- `ssp proxy fly --fly-app <app>` runs the arms of experiments as Machines of a Fly.io
  app. Each unit is a copy of a live Machine with the unit's image, without services,
  mounts, or `fly_*` metadata, so it gets no public traffic and `fly deploy` leaves it
  alone. The proxy reaches units on the private network.
- The runtime tests also run `ssp proxy fly` on a real Fly.io app.

### Fixed

- Cloud Run: a deploy of a new live gets all the traffic again while units run. The
  proxy pins live's traffic only while it makes the revision of a unit. Then it gives
  the service its own template and traffic, and adds only the unit's tag.
- Cloud Run: the proxy reads the output of all units with one Cloud Logging query
  every 5 seconds, not one query for each unit. When Cloud Logging answers 429, the
  proxy waits as it asks, or longer each time, and loses no line.
- Cloud Run: the proxy sends an ID token for the service's URL. Cloud Run refuses an
  ID token for the tag URL at the tag URL.

### Changed

- Pushes run only the fast tests. The runtime tests (Docker, Kubernetes, real Cloud
  Run, real ECS Fargate, and real Fly.io) run before each release, and an admin can
  run them on `main`.
- The example app listens on IPv6 and IPv4.

## 0.4.0 - 2026-10-29

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
  `max_share`, and its own events and metrics.
- `ssp experiment update <name> --arm <commit>=<share>` changes only the arms that it
  names. `=0` removes an arm. Shares are fractions: `5` and `5%` fail. It prints the
  change, the full list with the reference, and the total of `max_share`.
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
- `ssp proxy docker`, `ssp proxy cloudrun`, and `ssp proxy kubernetes`. One
  `Orchestrator` trait runs the units: one for each arm of each starting or running
  experiment. Every instance routes; the instance that holds the app's lease
  orchestrates.
- The proxy checks the running experiments in the order of `statespace.toml`. Each
  experiment reads its own subject. The first experiment whose arm range holds the
  subject's point takes the request. Else live serves it.
- Cloud Run: units run as revisions with 0% traffic and the tag
  `ssp-<experiment>-<commit 12>`. The proxy sends an ID token, and reads output from
  Cloud Logging.
- Kubernetes: the proxy runs as its own Deployment. Units run as copies of the app's
  Deployment, `<deployment>-ssp-<experiment>-<commit 12>`, each with a Service.
- Unit names that pass a platform limit end with a short hash. Labels keep the full
  experiment and commit.
- `[[event]]` rules from `statespace.toml`, checked in the proxy on requests, standard
  output, and standard error. A unit gets the shared events and the events of its
  experiment.
- The proxy records `runs`, `requests`, `stdout`, `stderr`, `events`, and `lifecycle`
  rows for units. Each row has the experiment, the arm, and the version.
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

### Removed

- `ssp apply`, `ssp status`, `ssp stop`, `ssp resume`, and their targets: pull
  request and repository URLs, paths, and `exp/` branches.
- Holds: `ssp app stop` and `resume`, `ssp arm stop` and `resume`.
- `ssp arm`. `ssp experiment update` sets the arms.
- `ssp init`. A person or a coding agent writes `statespace.toml` from the starter.
- The shared control, the `live` keyword, and the header `x-statespace-side`.
- The settings `max_traffic`, `max_arms`, and `arm_role`, and the app-wide `subject`
  and `eligibility`.
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
