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
- ❌ Edit Cargo.toml use `cargo add`
- ❌ Skip `cargo fmt`
- ❌ Merge without running clippy
- ❌ Comment self-evident operations (`// Initialize`, `// Return result`), getters/setters, constructors, or standard Rust idioms
- ❌ Add comments that restate what code does
- ❌ Make things optional that don't need to be - the compiler will enforce
- ❌ Add error context that doesn't add anything useful information (e.g., `.context("Failed to X")` when error already says it failed)

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

- Write self-documenting code - prefer clear names over comments

- Only comment for complex algorithms, non-obvious business logic, or "why" not "what"

- Booleans should default to false, not be optional

- Clean up existing logs, don't add more unless for errors or security events

- Avoid overly defensive code - trust Rust's type system

- Clean up existing logs, don't add more unless for errors or security events

## Entry Points
- CLI: binaries/statespace-cli/src/main.rs
- Server: crates/statespace-server/src/lib.rs
- Tool Runtime: crates/statespace-tool-runtime/src/lib.rs

## Design Documents

See [docs/design/](docs/design/) for design documents following the Oxide RFD style.

---

## Code Review Instructions

### Review Philosophy
- Only comment when you have HIGH CONFIDENCE (>80%) that an issue exists
- Be concise: one sentence per comment when possible
- Focus on actionable feedback, not observations
- When reviewing text, only comment on clarity issues if the text is genuinely confusing or could lead to errors. "Could be clearer" is not the same as "is confusing" - stay silent unless HIGH confidence it will cause problems

### Priority Areas (Review These)

#### Security & Safety
- Unsafe code blocks without justification
- Command injection risks (shell commands, user input)
- Path traversal vulnerabilities
- Credential exposure or hardcoded secrets
- Missing input validation on external data
- Improper error handling that could leak sensitive info

#### Correctness Issues
- Logic errors that could cause panics or incorrect behavior
- Race conditions in async code
- Resource leaks (files, connections, memory)
- Off-by-one errors or boundary conditions
- Incorrect error propagation (using `unwrap()` inappropriately)
- Optional types that don't need to be optional
- Booleans that should default to false but are set as optional
- Error context that doesn't add useful information (e.g., `.context("Failed to do X")` when error already says it failed)
- Overly defensive code that adds unnecessary checks
- Unnecessary comments that just restate what the code already shows (remove them)

#### Architecture & Patterns
- Code that violates existing patterns in the codebase
- Missing error handling (should use `anyhow::Result`)
- Async/await misuse or blocking operations in async contexts
- Improper trait implementations

#### No Prerelease Docs
- If the PR contains both code changes to features/functionality AND updates in `/docs`: Documentation updates must be separated to keep public docs in sync with released versions. Either mark new topics with `unlisted: true` or remove/hide the documentation.

### Project-Specific Context

- This is a Rust project using cargo workspaces
- Error handling: Use `anyhow::Result`, not `unwrap()` in production code
- Async runtime: tokio
- See HOWTOAI.md for AI-assisted code standards

### CI Pipeline Context

**Important**: You review PRs immediately, before CI completes. Do not flag issues that CI will catch.

#### What Our CI Checks (`.github/workflows/ci.yml`)

**Rust checks:**
- `cargo check --workspace` - Code compiles against stable Rust toolchain
- `cargo fmt --all -- --check` - Code formatting (rustfmt)
- `cargo clippy --workspace -- -D warnings` - Linting (clippy)
- `cargo test --workspace` - All tests

**Setup steps CI performs:**
- Checks out the code - actions/checkout@v4
- Installs Rust toolchain - dtolnay/rust-toolchain@stable (or @1.85.0 for MSRV)

## Skip These (Low Value)

Do not comment on:
- **Style/formatting** - CI handles this (rustfmt, prettier)
- **Clippy warnings** - CI handles this (clippy)
- **Test failures** - CI handles this (full test suite)
- **Missing dependencies** - CI handles this (npm ci will fail)
- **Minor naming suggestions** - unless truly confusing
- **Suggestions to add comments** - for self-documenting code
- **Refactoring suggestions** - unless there's a clear bug or maintainability issue
- **Multiple issues in one comment** - choose the single most critical issue
- **Logging suggestions** - unless for errors or security events (the codebase needs less logging, not more)
- **Pedantic accuracy in text** - unless it would cause actual confusion or errors. No one likes a reply guy

## Response Format

When you identify an issue:
1. **State the problem** (1 sentence)
2. **Why it matters** (1 sentence, only if not obvious)
3. **Suggested fix** (code snippet or specific action)

Example:
```
This could panic if the vector is empty. Consider using `.get(0)` or add a length check.
```

## When to Stay Silent

If you're uncertain whether something is an issue, don't comment. False positives create noise and reduce trust in the review process.
