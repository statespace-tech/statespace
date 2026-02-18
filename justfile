release version:
    #!/usr/bin/env bash
    set -euo pipefail

    VERSION="{{version}}"

    if [[ "$VERSION" =~ ^(major|minor|patch)$ ]]; then
        CURRENT=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
        IFS='.' read -r MAJOR MINOR PATCH <<< "${CURRENT%%-*}"
        
        case "$VERSION" in
            major) VERSION="$((MAJOR + 1)).0.0" ;;
            minor) VERSION="${MAJOR}.$((MINOR + 1)).0" ;;
            patch) VERSION="${MAJOR}.${MINOR}.$((PATCH + 1))" ;;
        esac
        echo "Bumping $CURRENT → $VERSION"
    fi

    [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]] || \
        { echo "error: invalid version (use semver, e.g. 0.1.0, or: major, minor, patch)"; exit 1; }

    git rev-parse "v${VERSION}" >/dev/null 2>&1 && \
        { echo "error: tag v${VERSION} already exists locally"; exit 1; }

    git ls-remote --tags origin "refs/tags/v${VERSION}" | grep -q . && \
        { echo "error: tag v${VERSION} already exists on remote"; exit 1; }

    git diff --quiet HEAD || { echo "error: uncommitted changes"; exit 1; }

    BRANCH=$(git rev-parse --abbrev-ref HEAD)
    [[ "$BRANCH" == "main" ]] || { echo "error: releases must be from main (on $BRANCH)"; exit 1; }

    git fetch origin main --quiet
    LOCAL=$(git rev-parse HEAD)
    REMOTE=$(git rev-parse origin/main)
    [[ "$LOCAL" == "$REMOTE" ]] || { echo "error: main is not up to date with origin/main"; exit 1; }

    echo ""
    echo "Release v${VERSION}"
    echo ""
    echo "This will:"
    echo "  1. Update version in Cargo.toml"
    echo "  2. Commit and tag v${VERSION}"
    echo "  3. Push to origin (triggers CI)"
    echo ""
    echo "CI will then:"
    echo "  - Publish crates to crates.io"
    echo "  - Build CLI binaries"
    echo "  - Create GitHub Release"
    echo ""
    read -p "Continue? [y/N] " c && [[ "$c" =~ ^[Yy]$ ]] || exit 0

    cargo set-version --workspace "${VERSION}"

    git add Cargo.toml Cargo.lock
    git commit -m "chore: release v${VERSION}"
    git tag "v${VERSION}"
    git push origin main "v${VERSION}"

    echo ""
    echo "Release triggered: https://github.com/statespace-tech/statespace/actions"
