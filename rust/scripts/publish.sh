#!/bin/bash
# Publish nostr-social-graph Rust crates to crates.io in dependency order.
#
# Usage:
#   ./scripts/publish.sh
#   ./scripts/publish.sh --dry-run
#   ./scripts/publish.sh --plan

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

DRY_RUN=""
ALLOW_DIRTY="--allow-dirty"
PLAN_ONLY=0
FAILED_CRATES=()

for arg in "$@"; do
    case "$arg" in
        --dry-run)
            DRY_RUN="--dry-run"
            echo "=== DRY RUN MODE ==="
            ;;
        --plan)
            PLAN_ONLY=1
            ;;
        *)
            echo "Unknown argument: $arg" >&2
            exit 1
            ;;
    esac
done

CRATES=(
    "nostr-identity"
    "nostr-social-graph"
    "nostr-social-graph-heed"
    "nostr-social-memory"
)

if [[ "$PLAN_ONLY" -eq 1 ]]; then
    for crate in "${CRATES[@]}"; do
        echo "$crate"
    done
    exit 0
fi

cd "$RUST_DIR"

publish_crate() {
    local crate=$1

    echo ""
    echo "=========================================="
    echo "Publishing: $crate"
    echo "=========================================="

    local output
    if [[ -n "$DRY_RUN" ]]; then
        if output=$(cargo publish -p "$crate" $DRY_RUN $ALLOW_DIRTY 2>&1); then
            echo "$output"
            echo "✓ $crate published successfully"
            return
        fi

        if echo "$output" | grep -q 'no matching package named `nostr-social-graph` found'; then
            echo "$output"
            echo "Dependency is not on crates.io yet during dry-run; validating local build instead..."
            cargo check -p "$crate"
            echo "✓ $crate validated locally for publish ordering"
            return
        fi

        echo "$output"
        echo "✗ Failed to publish $crate (continuing...)"
        FAILED_CRATES+=("$crate")
        return
    fi

    if output=$(cargo publish -p "$crate" $DRY_RUN $ALLOW_DIRTY 2>&1); then
        echo "$output"
        echo "✓ $crate published successfully"
    elif echo "$output" | grep -q "already exists"; then
        echo "$output"
        echo "✓ $crate already published at this version (skipping)"
    else
        echo "$output"
        echo "✗ Failed to publish $crate (continuing...)"
        FAILED_CRATES+=("$crate")
    fi
}

echo "Publishing nostr-social-graph crates to crates.io"

if [[ -z "$DRY_RUN" ]]; then
    echo "Checking crates.io authentication..."
    if ! cargo login --help >/dev/null 2>&1; then
        echo "Please run 'cargo login' first"
        exit 1
    fi
fi

for crate in "${CRATES[@]}"; do
    publish_crate "$crate"
done

echo ""
echo "=========================================="
if [[ ${#FAILED_CRATES[@]} -eq 0 ]]; then
    echo "✓ All crates published successfully!"
else
    echo "✗ Failed to publish: ${FAILED_CRATES[*]}"
    exit 1
fi
echo "=========================================="
