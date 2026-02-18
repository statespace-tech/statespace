set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# Bump version, commit, tag, and push — then trigger the release workflow.
# level: patch | minor | major
# Example: just release patch
release level:
    #!/usr/bin/env bash
    set -euo pipefail

    # Bump version + commit + tag (no publish — CI handles that)
    cargo release --workspace --no-publish --execute {{level}}

    TAG=$(git describe --tags --abbrev=0)
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
