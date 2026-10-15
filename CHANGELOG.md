# Changelog

Notable changes to Statespace and its `ssp` CLI. Releases follow semantic versioning.

## 0.1.0 - Unreleased

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
- An agent skill in `skills/statespace`.
