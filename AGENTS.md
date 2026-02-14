# AGENTS.md - Statespace

## Overview

Statespace is an open-source AI runtime/framework. This monorepo contains the Rust implementation.

## Build/Test Commands

- **Build**: `cargo build`
- **Test**: `cargo test`
- **Lint**: `cargo clippy --all-targets -- -D warnings`
- **Format**: `cargo fmt --all`
- **Check**: `cargo check`

## Pre-commit Hooks

Pre-commit hooks run `cargo fmt --check` and `cargo clippy`. To enable:

```bash
git config core.hooksPath .githooks
```

**Note**: On macOS, the linker needs to find `libiconv` (pulled in by reqwest/rustls). The `.envrc` handles this automatically with `direnv`. Without direnv, set:
```bash
export LIBRARY_PATH="$(xcrun --show-sdk-path)/usr/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
```

## Project Structure

```
statespace/
├── Cargo.toml                      # Workspace manifest
├── shell.nix                       # Nix development environment
├── binaries/
│   └── statespace-cli/             # CLI binary
│       └── src/
│           ├── main.rs
│           ├── args.rs
│           ├── config.rs
│           ├── error.rs
│           ├── commands/
│           └── gateway/
├── crates/
│   ├── statespace-tool-runtime/    # Core runtime library (no HTTP, no CLI)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs
│   │       ├── executor.rs
│   │       ├── frontmatter.rs
│   │       ├── protocol.rs
│   │       ├── security.rs
│   │       ├── spec.rs
│   │       ├── tools.rs
│   │       └── validation.rs
│   └── statespace-server/          # HTTP server library
│       └── src/
│           ├── lib.rs
│           ├── content.rs
│           ├── error.rs
│           ├── init.rs
│           ├── server.rs
│           └── templates.rs
└── docs/
    └── design/                     # RFDs
```

## Architecture

This is a **stateless CLI** — the gateway's actor model, sagas, and stateful lifecycle management do **not** apply here. There are no long-lived processes, no message passing, no state machines.

**What DOES apply from gateway philosophy:**
- **Type-driven design** — validated enums, newtypes, sum types. Make invalid states unrepresentable.
- **Effects at the edges** — the `gateway/` module inside `statespace-cli` is the effect boundary (HTTP calls to the Statespace API). Commands are orchestration. Pure logic (validation, slugify, config parsing) must have no I/O.
- **Explicit dependencies** — pass config and clients explicitly; no globals, no `lazy_static!`, no hidden env var reads.
- **One file per concept** — not `models.rs` dumping grounds.
- **Immutable data with methods** — domain structs are values with pure business logic methods.

**Module roles:**
- **Pure modules** (no I/O): `frontmatter`, `spec`, `security`, `protocol`, `validation`, `templates`
- **Effectful edge**: `executor`, `content`, `server`, `init`, `gateway/`
- **Commands**: thin orchestration — parse args, call gateway, format output

### Code Organization

- **One file per domain** in `gateway/`: `environments.rs`, `auth.rs`, `tokens.rs`, `organizations.rs`, `ssh.rs` — each file owns the types and API calls for that domain
- **No `#![allow(dead_code)]` file-level suppression** — if code is dead, delete it. Mark individual items with `#[allow(dead_code)]` only with a comment explaining why.
- **Pure functions in dedicated modules** — validation, parsing, slug generation go in their own modules, not inline in command handlers
- **Types live with their domain** — an `Environment` struct belongs in `gateway/environments.rs`, not in a shared `types.rs`

### Anti-Patterns

- ❌ `types.rs` / `models.rs` dumping grounds — split types by domain, co-locate with their API calls
- ❌ `status: String` when the server defines a proper enum — use typed enums (`EnvironmentState`, `Tier`, `Visibility`) client-side too
- ❌ `#![allow(dead_code)]` to hide unused code — delete it, or annotate individual items with a justification
- ❌ Inline validation/parsing in command handlers — extract to pure functions that can be unit tested
- ❌ Hidden I/O in "pure" modules — if it touches the network or filesystem, it belongs at the edge

### Dependency Graph

```
statespace-cli ──► statespace-server ──► statespace-tool-runtime
       │                                          ▲
       └──────────────────────────────────────────┘
```

## Rust Code Guidelines

- Do NOT use `unwrap()` or `expect()` or anything that panics in library code - handle errors properly. In tests, `unwrap()` and `panic!()` are fine.

- Prefer `crate::` over `super::` for imports. Clean it up if you see `super::`.

- Avoid using `pub use` on imports unless you are re-exposing a dependency so downstream consumers do not have to depend on it directly.

- Skip global state via `lazy_static!`, `Once`, or similar; prefer passing explicit context structs for any shared state.

## Design Documents

See [docs/design/](docs/design/) for design documents following the Oxide RFD style.
