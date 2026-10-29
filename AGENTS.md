# Contributor instructions

An experiment compares commits of one app. Each arm is one commit with an exact share
of the app's subjects. `ssp app`, `ssp experiment`, and `ssp token` call the Statespace
API. `ssp proxy docker`, `ssp proxy cloudrun`, and `ssp proxy kubernetes` run a reverse
proxy in front of the app's live service. Every proxy instance routes requests to the
arms of running experiments, in the order of `statespace.toml`. Only the instance that
holds the app's lease starts and stops units, one for each arm of each starting or
running experiment, through the `Orchestrator` trait in `src/proxy/orchestrator`.

`../design.md` is the source of truth for the model.

## Rules

- Use `ssp <noun> <verb> <object>`. The object is positional. `--app` names the app of an
  experiment or a token. Leave it optional where the folder can tell the app.
- Print text by default and exactly one JSON document with `--json`, except `ssp proxy`.
  Send notes and warnings to standard error. Errors exit 1, as `{"error": "..."}` with
  `--json`.
- Give `--note` (1 to 200 characters, one line) to every command that changes something.
- Keep settings in `statespace.toml`. Add no proxy flag for a setting.
- Keep commands noninteractive, except `ssp login`.
- Print secrets only in the response that creates them. Never pass `STATESPACE_*`
  variables to units or live.
- `ssp proxy` must never block or slow the app: drop rows rather than wait, and send a
  request to live rather than fail it.
- Never record live.
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
```

## Commits

Use Conventional Commits, such as `feat(proxy): restart units with backoff`. Add no AI
attribution.
