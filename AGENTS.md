# Contributor instructions

Experiments run from pull requests of `exp/` branches. `ssp apply` records the image that
CI built, `ssp status`, `ssp stop`, and `ssp resume` act on pull requests, and settings
live in `statespace.toml`. `ssp proxy` is a reverse proxy that assigns each request's
subject and forwards it to the app's default or an experiment's arm; on Docker it also
runs them as containers. `ssp controller kubernetes` runs the arms on Kubernetes.

## Rules

- Print text by default and exactly one JSON document with `--json`, except `ssp proxy`
  and `ssp controller`. Send notes and warnings to standard error. Errors exit 1, as
  `{"error": "..."}` with `--json`.
- Change settings through commits to `statespace.toml`, not commands. Read data through
  SQL. Add no command that only reads recorded data.
- Keep commands noninteractive, except `ssp login`.
- Print secrets only in the response that creates them.
- Require `--yes` for irreversible commands.
- `ssp proxy` must never block or slow the app: drop events rather than wait, and send a
  subject to the default rather than fail its request.
- Run only images that Statespace recorded from the repository's CI: arms of pull requests
  that can merge, and controls whose image a proxy or controller ran as the default.
- Never record the default.
- Never change the bucket function: it would move subjects of running experiments.

## Checks

```shell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
sh -n install.sh && shellcheck install.sh
STATESPACE_DOCKER_TESTS=1 cargo test --locked --test cli docker   # needs Docker
```

## Commits

Use Conventional Commits, such as `feat(proxy): restart arms with backoff`.
