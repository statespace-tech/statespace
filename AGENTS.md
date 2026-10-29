# Contributor instructions

An arm is a commit that runs on a share of an app's live traffic, beside a control: a
copy of the image that live runs. `ssp app`, `ssp arm`, and `ssp token` call the
Statespace API. `ssp init` writes a starter `statespace.toml`. `ssp proxy docker`,
`ssp proxy cloudrun`, and `ssp proxy kubernetes` run a reverse proxy in front of the
app's live service. Every proxy instance routes. Only the instance that holds the app's
lease starts and stops the control and the arms, through the `Orchestrator` trait in
`src/proxy/orchestrator`.

`../design.md` is the source of truth for the model.

## Rules

- Use `ssp <noun> <verb> <object>`. The object is positional. `--app` names the app of an
  arm or a token.
- Print text by default and exactly one JSON document with `--json`, except `ssp proxy`.
  Send notes and warnings to standard error. Errors exit 1, as `{"error": "..."}` with
  `--json`.
- Give `--note` (1 to 200 characters, one line) to every command that changes something.
- Keep settings in `statespace.toml`. Add no proxy flag for a setting.
- Keep commands noninteractive, except `ssp login`.
- Print secrets only in the response that creates them. Never pass `STATESPACE_*`
  variables to the control, the arms, or live.
- `ssp proxy` must never block or slow the app: drop rows rather than wait, and send a
  request to live rather than fail it.
- Never record live.
- Never change the bucket function: it would move the subjects of running arms.
- Keep no legacy: when the design changes, remove the old way.
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
