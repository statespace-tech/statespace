set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# Regenerate docs/pages/reference/cli.md from the CLI's clap definitions
gen-docs:
    cargo run --bin gen-cli-docs

# Bump version, commit, tag, and push — then trigger the release workflow.
# level: patch | minor | major
# Example: just release patch
release level:
    #!/usr/bin/env bash
    set -euo pipefail

    PREV_HEAD=$(git rev-parse HEAD)
    PREV_TAG=$(git describe --tags --abbrev=0)

    # Bump version + commit + tag (no publish — CI handles that)
    cargo release --workspace --no-publish --no-confirm --execute {{level}}

    NEW_HEAD=$(git rev-parse HEAD)
    TAG=$(git describe --tags --abbrev=0)

    if [ "$NEW_HEAD" = "$PREV_HEAD" ] || [ "$TAG" = "$PREV_TAG" ]; then
        echo "Release did not create a new commit and tag; refusing to push or trigger CI."
        exit 1
    fi

    echo "Tag: ${TAG}"

    git push origin main --follow-tags

    echo "Triggering release workflow for ${TAG}..."
    gh workflow run release.yml --ref "${TAG}" -f tag="${TAG}"
    echo "Monitor: gh run list --workflow=release.yml"

# Trigger a dry-run build of the release workflow (no publish, no GitHub release)
release-dry-run ref="main":
    gh workflow run release.yml --ref {{ref}} -f tag="dry-run"
    echo "Triggered dry-run release workflow (ref: {{ref}})"
    echo "Monitor: gh run list --workflow=release.yml"

# Trigger a release for an existing tag (e.g. to retry a failed run)
release-tag tag ref="main":
    gh workflow run release.yml --ref {{ref}} -f tag="{{tag}}"
    echo "Triggered release workflow for {{tag}} (ref: {{ref}})"
    echo "Monitor: gh run list --workflow=release.yml"

# Watch recent release workflow runs
watch-release:
    gh run list --workflow=release.yml --limit 10
