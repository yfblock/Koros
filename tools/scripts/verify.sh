#!/bin/bash
# Run Verus formal verification on all per-crate verification modules.
#
# Verification code lives next to the code it models: each crate keeps its
# Verus-annotated copies under `<crate>/verify/*.rs`. These files are
# standalone (not part of the cargo build); Verus checks them directly.
#
# Usage:
#   ./scripts/verify.sh              # verify every <crate>/verify/*.rs
#   ./scripts/verify.sh slab_heap    # verify by module name (searched across crates)

set -euo pipefail

VERUS="${VERUS:-/tmp/verus-release/verus-x86-linux/verus}"
ROOT="$(dirname "$0")/../.."

if [ ! -x "$VERUS" ]; then
    echo "Error: Verus not found at $VERUS"
    echo "Set VERUS env var or install Verus to /tmp/verus-release/"
    exit 1
fi

if [ $# -gt 0 ]; then
    FILES=()
    for name in "$@"; do
        matches=$(find "$ROOT" -mindepth 3 -maxdepth 4 -path "*/verify/${name}.rs" -not -path "*/polyhal/*" -not -path "*/target/*")
        if [ -z "$matches" ]; then
            echo "Error: no <crate>/verify/${name}.rs found"
            exit 1
        fi
        # shellcheck disable=SC2206
        FILES+=($matches)
    done
else
    # shellcheck disable=SC2206
    FILES=$(find "$ROOT" -mindepth 3 -maxdepth 4 -path "*/verify/*.rs" -not -path "*/polyhal/*" -not -path "*/target/*")
fi

errors=0
for f in $FILES; do
    echo "=== Verifying ${f#"$ROOT"/} ==="
    if "$VERUS" --triggers-mode silent "$f" 2>&1; then
        echo "PASS: $(basename "$f")"
    else
        echo "FAIL: $(basename "$f")"
        errors=$((errors + 1))
    fi
    echo
done

if [ $errors -gt 0 ]; then
    echo "$errors file(s) failed verification"
    exit 1
fi
echo "All files verified successfully"
