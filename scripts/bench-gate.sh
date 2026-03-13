#!/usr/bin/env bash
# bench-gate.sh - Performance regression gate for drift benchmarks.
#
# Compares current benchmark results against a saved baseline and fails
# if any benchmark regresses beyond the configured threshold.
#
# Usage:
#   ./scripts/bench-gate.sh              # run benchmarks then compare against baseline
#   ./scripts/bench-gate.sh --save       # save current run as baseline
#   ./scripts/bench-gate.sh --compare    # compare existing baseline vs current (no bench run)
#   BENCH_THRESHOLD=20 ./scripts/bench-gate.sh  # custom threshold (%)
#
# Prerequisites:
#   cargo install critcmp

set -euo pipefail

THRESHOLD="${BENCH_THRESHOLD:-15}"
BASELINE_NAME="baseline"
CURRENT_NAME="current"
BENCH_TARGET="drift"
BENCH_PACKAGE="am-core"

bold() { printf '\033[1m%s\033[0m\n' "$1"; }
red()  { printf '\033[31m%s\033[0m\n' "$1"; }
green(){ printf '\033[32m%s\033[0m\n' "$1"; }

if ! command -v critcmp &>/dev/null; then
    red "critcmp not found. Install with: cargo install critcmp"
    exit 1
fi

MODE="${1:-run}"

if [[ "$MODE" == "--save" ]]; then
    bold "Saving benchmark baseline..."
    cargo bench -p "$BENCH_PACKAGE" --bench "$BENCH_TARGET" -- --save-baseline "$BASELINE_NAME"
    green "Baseline saved. Run without --save to compare future runs."
    exit 0
fi

# Verify baseline exists
BASELINE_DIR="target/criterion"
baseline_found=false
for dir in "$BASELINE_DIR"/*/; do
    if [[ -d "${dir}${BASELINE_NAME}" ]]; then
        baseline_found=true
        break
    fi
done

if [[ "$baseline_found" == "false" ]]; then
    green "No baseline data found in $BASELINE_DIR. Skipping comparison (first run)."
    exit 0
fi

# In --compare mode, skip the bench run (CI pre-runs both baselines)
if [[ "$MODE" != "--compare" ]]; then
    bold "Running benchmarks..."
    cargo bench -p "$BENCH_PACKAGE" --bench "$BENCH_TARGET" -- --save-baseline "$CURRENT_NAME"
fi

bold "Comparing results (threshold: ${THRESHOLD}%)..."
COMPARISON=$(critcmp "$BASELINE_NAME" "$CURRENT_NAME" 2>&1) || true
echo "$COMPARISON"

regression_found=false
while IFS= read -r line; do
    if pct=$(echo "$line" | grep -oE '[+-][0-9]+(\.[0-9]+)?%' | head -1); then
        if [[ -n "$pct" ]]; then
            num=$(echo "$pct" | sed 's/[%+]//g')
            sign=$(echo "$pct" | cut -c1)
            if [[ "$sign" == "+" ]]; then
                is_regression=$(awk "BEGIN { print ($num > $THRESHOLD) ? 1 : 0 }")
                if [[ "$is_regression" == "1" ]]; then
                    regression_found=true
                    red "REGRESSION: $line (exceeds ${THRESHOLD}% threshold)"
                fi
            fi
        fi
    fi
done <<< "$COMPARISON"

if [[ "$regression_found" == "true" ]]; then
    echo ""
    red "Performance regression detected. Gate FAILED."
    red "If the regression is intentional, update the baseline: just bench-baseline"
    exit 1
else
    echo ""
    green "No regressions beyond ${THRESHOLD}% threshold. Gate PASSED."
    exit 0
fi
