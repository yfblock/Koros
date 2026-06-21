#!/bin/bash
# Run Verus formal verification on all verified modules.
#
# Usage:
#   ./verify.sh              # verify all
#   ./verify.sh frame_allocator  # verify single file

set -euo pipefail

VERUS="${VERUS:-/tmp/verus-release/verus-x86-linux/verus}"
VERIFIED_DIR="$(dirname "$0")/../verified"

if [ ! -x "$VERUS" ]; then
    echo "Error: Verus not found at $VERUS"
    echo "Set VERUS env var or install Verus to /tmp/verus-release/"
    exit 1
fi

if [ $# -gt 0 ]; then
    FILES=()
    for name in "$@"; do
        f="$VERIFIED_DIR/${name}.rs"
        if [ ! -f "$f" ]; then
            echo "Error: $f not found"
            exit 1
        fi
        FILES+=("$f")
    done
else
    FILES=("$VERIFIED_DIR"/*.rs)
fi

errors=0
for f in "${FILES[@]}"; do
    echo "=== Verifying $(basename "$f") ==="
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
