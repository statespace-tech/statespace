# Changelog

Notable changes to Statespace and its `ssp` CLI. Releases follow semantic versioning.

## Unreleased

Experiments run from pull requests. A branch named `exp/<experiment>/<arm>` is an arm,
and it runs while GitHub says that its pull request can merge. This release removes the
terminal flow of 0.1.0.

### Added

- `ssp init github` writes `statespace.toml` and a workflow that publishes each commit of
  an `exp/` branch.
- `statespace.toml` gives the app, and an arm's share, subject rule, and eligibility rule.
  `ssp check` tests it and a request.
- `ssp apply` records a CI job's image with the job's GitHub OIDC token. On the default
  branch, the image becomes a default build; on an `exp/` branch, a version of the arm.
- `ssp status` shows experiments, their arms and controls, and their pull requests.
- `ssp stop` stops arms at once and makes their pull requests drafts.
- `ssp repo connect` and `ssp repo list` install the Statespace GitHub App.
- `ssp app set --max-traffic` limits the share of an app's subjects in experiments.
- Each base commit of an experiment's arms has its own control.

### Changed

- `ssp proxy` runs containers with `--image` or `--image-file`, and only routes with
  `--default`. It reports the digest of its default.
- `ssp controller` is `ssp controller kubernetes`.
- `--max-variants` is `--max-arms`, and `--variant-env` is `--arm-env`.
- Events, views, headers, and container variables name an `arm` and its `commit`:
  `x-statespace-arm`, `STATESPACE_ARM`, and `STATESPACE_COMMIT`.
- `ssp login` signs in with GitHub only.
- The controller preset's scope `variants:report` is `arms:report`.

### Removed

- `ssp variant`, `ssp experiment`, `ssp keygen`, `ssp app create`, `--runtime`,
  `--trust-key`, signing keys, and the `publisher` token preset.

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
