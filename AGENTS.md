# AGENTS.md - Statespace

## Overview

Statespace is an open-source AI tool execution runtime. This monorepo contains the Rust implementation.

## Build/Test Commands

- **Build**: `cargo build`
- **Test**: `cargo test`
- **Lint**: `cargo clippy`
- **Format**: `cargo fmt`
- **Check**: `cargo check`

**Note**: On macOS, run inside `nix-shell` to get `libiconv` linked properly:
```bash
nix-shell --run "cargo build"
nix-shell --run "cargo test"
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
├── legacy/                         # Deprecated Python implementation
│   ├── src/toolfront/
│   ├── tests/
│   └── pyproject.toml
└── docs/
    └── design/                     # RFDs
```

## Architecture

Follows FP-Rust patterns:
- **Pure modules** (no I/O): `frontmatter`, `spec`, `security`, `protocol`, `validation`, `templates`
- **Effectful edge**: `executor`, `content`, `server`, `init`

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

## CLI Commands

```bash
# Serve a tool site locally
cargo run -p statespace-cli -- app serve /path/to/toolsite

# Deploy to cloud
cargo run -p statespace-cli -- app deploy /path/to/toolsite --name myapp

# List apps
cargo run -p statespace-cli -- app list

# Token management
cargo run -p statespace-cli -- tokens list
```

## Design Documents

See [docs/design/](docs/design/) for design documents following the Oxide RFD style.
