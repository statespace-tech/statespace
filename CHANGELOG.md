# Changelog

Notable changes to Statespace and its `ssp` CLI. Releases follow semantic versioning.

## Unreleased

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
