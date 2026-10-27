#!/bin/sh
# Local gate: the same checks the CI workflow runs.
#
#   ./scripts/check.sh
#
# Exits non-zero on the first failure. Steps that need a tool which is not
# installed are reported and skipped, so the script works on a bare checkout.
set -eu

cd "$(dirname "$0")/.."

step() {
    printf '\n==> %s\n' "$1"
}

have() {
    command -v "$1" >/dev/null 2>&1
}

step "cargo fmt"
cargo fmt --all --check

step "clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "rust tests (CLI crate, as in CI)"
cargo test --release -p one-note-converter

if [ -x python/.venv/bin/python ]; then
    step "python extension"
    (cd python && .venv/bin/maturin develop --release)
    step "python tests"
    (cd python && .venv/bin/python -m pytest tests -q)
else
    step "python tests skipped: python/.venv is missing (see python/README.md)"
fi

if [ -x python/.venv/bin/maturin ]; then
    step "packaging: build wheel + sdist"
    rm -rf dist
    (cd python && .venv/bin/maturin build --release --sdist --out ../dist)
    if [ -x python/.venv/bin/twine ]; then
        step "packaging: metadata check"
        python/.venv/bin/twine check dist/*
    else
        step "packaging: twine not installed, skipping metadata check"
    fi
fi

if have actionlint; then
    step "workflows"
    actionlint
else
    step "workflows skipped: actionlint not installed"
fi

step "done"
