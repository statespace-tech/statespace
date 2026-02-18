# Release a new version. Requires cargo-release: cargo install cargo-release
release level:
    cargo release --workspace --no-publish --execute {{level}}
