#!/usr/bin/env bash
# What CI would say, said here before a push, while GitHub Actions is off.
#
#   scripts/check.sh            the crates changed since origin/main, and the workspace build
#   scripts/check.sh --all      every crate, as .github/workflows/ci.yml runs them
#   CLIPPY=1 scripts/check.sh   also clippy -D warnings on the changed crates (slow)
#
# Twice now a commit that did not compile reached main, and everyone pulling it was stopped
# by it. This runs the same checks CI runs, on what changed, and exits non-zero if any
# fails, so it can sit in a pre-push hook:
#
#   printf '#!/bin/sh\nexec scripts/check.sh\n' > .git/hooks/pre-push && chmod +x .git/hooks/pre-push
#
# Environment: BASE (default origin/main), CARGO_TARGET_DIR, JOBS (default 2).
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root/core"
base=${BASE:-origin/main}
jobs=${JOBS:-2}

if [[ "${1:-}" == "--all" ]]; then
    crates=()
    all=1
else
    all=0
    mapfile -t crates < <(
        {
            git diff --name-only "$base"...HEAD -- . 2>/dev/null || true
            git diff --name-only -- .
            git diff --name-only --cached -- .
        } | sed -n 's|^core/crates/\([^/]*\)/.*|\1|p' | sort -u
    )
fi

step() { printf '\n== %s\n' "$*"; }
failed=0
run() { "$@" || failed=1; }

if [[ $all == 1 ]]; then
    step "format (all)"
    run cargo fmt --all -- --check
else
    for crate in "${crates[@]}"; do
        step "format $crate"
        run cargo fmt -p "$crate" -- --check
    done
fi

step "the whole workspace builds, with the lock file as committed"
run cargo check -j "$jobs" --workspace --all-targets --locked

if [[ $all == 1 ]]; then
    step "tests (all)"
    run cargo test -j "$jobs" --workspace --locked
else
    for crate in "${crates[@]}"; do
        step "tests $crate"
        run cargo test -j "$jobs" -p "$crate" --locked
    done
fi

if [[ "${CLIPPY:-0}" == 1 ]]; then
    for crate in "${crates[@]}"; do
        step "clippy $crate"
        run cargo clippy -j "$jobs" -p "$crate" --all-targets --locked -- -D warnings
    done
fi

if [[ $failed != 0 ]]; then
    printf '\ncheck: something failed above; fix it before pushing.\n' >&2
    exit 1
fi
printf '\ncheck: ok (%s)\n' "${crates[*]:-all crates}"
