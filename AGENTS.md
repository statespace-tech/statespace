# Contributor instructions

An experiment compares commits of one app. Each arm is one commit with an exact share
of the app's subjects. `ssp app`, `ssp experiment`, and `ssp token` call the
Statespace API. `ssp proxy docker`, `ssp proxy kubernetes`, `ssp proxy fargate`, `ssp proxy
cloudrun`, `ssp proxy fly`, and `ssp proxy lambda` run a reverse proxy in front of the
app's live service.
`ssp proxy s3` and `ssp proxy gcs` serve a static site from a bucket: live is the
bucket root, and each arm is a build folder (`src/proxy/orchestrator/files`).
Every proxy instance routes requests to the arms of running experiments, in the order
of `statespace.toml`. Only the instance that holds the app's lease starts and stops
units, one for each arm of each starting or running experiment, through the
`Orchestrator` trait in `src/proxy/orchestrator`. On a file runtime, every instance
checks the build folders, because each instance serves the files itself.

`../design/` is the source of truth for the model.

## Rules

- Use `ssp <noun> <verb> <object>`. The object is positional. `--app` names the app of an
  experiment or a token. Leave it optional where the folder can tell the app. `ssp proxy`
  takes no `--app`: Statespace tells the app of the runtime token.
- Run alongside live, never inside it: never change a resource that `ssp` did not
  create. Each unit is a resource that `ssp` creates, labels, and deletes.
- Print text by default and exactly one JSON document with `--json`, except `ssp proxy`.
  `--json` carries exactly the information of the text output, structured; never more.
  Send notes and warnings to standard error. Errors exit 1, as `{"error": "..."}` with
  `--json`.
- Give `--note` (1 to 200 characters, one line) to every command that changes something.
- Keep settings in `statespace.toml`. Add no proxy flag for a setting.
- Keep commands noninteractive, except `ssp login`.
- Print secrets only in the response that creates them. Never pass `STATESPACE_*`
  variables to units or live.
- `ssp proxy` must never block or slow the app: drop counts rather than wait, and send a
  request to live rather than fail it. Build an orchestrator from its flags alone, so the
  proxy serves live before Statespace names the app; the calls that report live or run
  units take the `App`.
- Never count live or overrides. Read no logs of units or live.
- Never change the bucket function: it would move the subjects of running experiments.
- Keep no legacy: when the design changes, remove the old way.
- Keep the starter `statespace.toml` and the CI step in `skills/statespace/SKILL.md`
  current. There is no `ssp init`.
- Write docs, READMEs, and skills in Simplified Technical English. Never name customers.

## Checks

```shell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
sh -n install.sh && shellcheck install.sh
# Needs Docker
STATESPACE_DOCKER_TESTS=1 cargo test --locked --test cli docker
# Needs a kind cluster selected in kubectl
STATESPACE_KIND_TESTS=1 cargo test --locked --test cli kind
# Needs a Fly.io test app and its deploy token; see CONTRIBUTING.md
STATESPACE_FLY_TESTS=1 cargo test --locked --test cli fly_proxies_run_arms
# Real AWS Fargate, Google Cloud Run, AWS Lambda, Amazon S3, and Google Cloud Storage:
# see CONTRIBUTING.md
```

## Commits

Use Conventional Commits, such as `feat(proxy): restart units with backoff`. Add no AI
attribution.
