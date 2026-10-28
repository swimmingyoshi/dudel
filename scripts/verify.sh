#!/usr/bin/env bash
# Development: ./scripts/verify.sh quick [test-filter ...]
# Release/CI: ./scripts/verify.sh full
set -euo pipefail
cd "$(dirname "$0")/.."
mode="${1:-quick}"
case "$mode" in quick|full) ;; *) echo 'Expected quick or full' >&2; exit 2;; esac
if [[ "$mode" == full && $# -gt 1 ]]; then echo 'Full verification does not accept test filters' >&2; exit 2; fi
# Turn on the shared pre-push format check for this clone.
if git rev-parse --git-dir >/dev/null 2>&1 && [[ -z "$(git config core.hooksPath || true)" ]]; then
    git config core.hooksPath .githooks
fi
cargo fmt --all -- --check
if [[ $# -gt 1 ]]; then cargo test --locked -- "${@:2}"; else cargo test --locked; fi
cargo clippy --locked --all-targets -- -D warnings
python3 -m unittest discover -s research -p 'test_*.py'
# No personal paths or private research in tracked files (the repository is public).
python3 scripts/check_public_tree.py
if [[ "$mode" == full ]]; then
    cargo test --locked --all-features
    cargo clippy --locked --all-targets --all-features -- -D warnings
    cargo build --locked --release
fi
