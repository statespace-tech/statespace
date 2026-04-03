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

# Dry-run the release pipeline on main and only cut a real release if it passes.
# Example: just release-safe patch
release-safe level:
    #!/usr/bin/env bash
    set -euo pipefail

    if ! git diff --quiet || ! git diff --cached --quiet; then
        echo "Release-safe requires a clean working tree."
        exit 1
    fi

    BRANCH=$(git branch --show-current)
    if [ "$BRANCH" != "main" ]; then
        echo "Release-safe must be run from main."
        exit 1
    fi

    git fetch origin main --quiet
    LOCAL_HEAD=$(git rev-parse HEAD)
    REMOTE_HEAD=$(git rev-parse origin/main)
    if [ "$LOCAL_HEAD" != "$REMOTE_HEAD" ]; then
        echo "main is not up to date with origin/main; pull or push before releasing."
        exit 1
    fi

    PREV_RUN_ID=$(gh run list --workflow=release.yml --limit 1 --json databaseId --jq '.[0].databaseId // ""')

    echo "Triggering dry-run release workflow on main..."
    gh workflow run release.yml --ref main -f tag="dry-run"

    RUN_ID=""
    for _ in {1..20}; do
        RUN_ID=$(gh run list --workflow=release.yml --limit 10 --json databaseId,event,headSha --jq ".[] | select(.event == \"workflow_dispatch\" and .headSha == \"$LOCAL_HEAD\") | .databaseId" | head -n1)
        if [ -n "$RUN_ID" ] && [ "$RUN_ID" != "$PREV_RUN_ID" ]; then
            break
        fi
        sleep 3
    done

    if [ -z "$RUN_ID" ] || [ "$RUN_ID" = "$PREV_RUN_ID" ]; then
        echo "Could not find the new dry-run release workflow."
        exit 1
    fi

    RUN_URL=$(gh run view "$RUN_ID" --json url --jq '.url')
    echo "Watching dry-run release workflow: ${RUN_URL}"
    gh run watch "$RUN_ID" --exit-status

    git fetch origin main --quiet
    if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then
        echo "main changed while the dry-run was running; pull and rerun release-safe."
        exit 1
    fi

    just release {{level}}

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
