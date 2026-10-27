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

# Python tooling lives in a venv; reuse python/.venv when present, otherwise
# build a throwaway one from python/requirements-dev.txt so the gate runs on a
# bare checkout.
VENV=""
if [ -x python/.venv/bin/python ]; then
    VENV=python/.venv
else
    VENV=.check-venv
    step "creating $VENV (python/.venv not found)"
    python3 -m venv "$VENV"
    "$VENV/bin/pip" install --quiet --upgrade pip
    "$VENV/bin/pip" install --quiet -r python/requirements-dev.txt
fi

step "cargo fmt"
cargo fmt --all --check

step "clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "rust tests (CLI crate, as in CI)"
cargo test --release -p one-note-converter

step "python extension"
(cd python && VIRTUAL_ENV="../$VENV" "../$VENV/bin/maturin" develop --release)

step "python tests"
(cd python && "../$VENV/bin/python" -m pytest tests -q)

step "packaging: wheel + sdist"
rm -rf dist
(cd python && VIRTUAL_ENV="../$VENV" "../$VENV/bin/maturin" build --release --sdist --out ../dist)

step "packaging: metadata check"
"$VENV/bin/twine" check dist/*

step "packaging: install the sdist into a clean venv"
rm -rf .check-sdist
python3 -m venv .check-sdist
.check-sdist/bin/pip install --quiet dist/*.tar.gz
.check-sdist/bin/python -c "import onenote; print('sdist import ok', onenote.__version__)"
rm -rf .check-sdist

if have actionlint; then
    step "workflows"
    actionlint
else
    step "workflows skipped: actionlint not installed"
fi

step "done"
