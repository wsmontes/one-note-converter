# Releasing

Three registries are fed from one tag. Everything below is one-time setup except
the version bump and the tag.

> **Status (v0.1.1):** the account billing lock is gone and the pipelines run.
> CI is green on `ubuntu-24.04`, `macos-15` and `windows-2025` (Rust) plus
> `Python 3.9`/`3.13`; the `v0.1.1` release built five wheels (manylinux
> x86_64/aarch64, macOS arm64/x86_64, Windows amd64) and the sdist, and attached
> them to the GitHub release. The Homebrew tap auto-bumped to `0.1.1` through its
> own workflow, and its daily schedule is back on.
>
> Left: publishing to the registries. PyPI needs a pending trusted publisher on
> <https://pypi.org/manage/account/publishing/> (project `onenote-converter`,
> owner `wsmontes`, repository `one-note-converter`, workflow `release.yml`,
> environment `pypi`) and then the repository variable `PYPI_PUBLISH=true`;
> crates.io needs the `CARGO_REGISTRY_TOKEN` secret. Both environments already
> exist, so only those two settings are missing. Until they are set the publish
> jobs are skipped and a tag run stays green. Build outputs (`dist/`) are
> gitignored — release assets are the distributables.

## 1. Version bump

The version lives in one place: `[workspace.package] version` in `Cargo.toml`.
`python/Cargo.toml` inherits it and maturin reads it for the wheel/sdist.

```sh
# edit Cargo.toml → version = "0.2.0"
cargo test --workspace --release
cd python && maturin develop --release && python -m pytest tests -q
```

Add the matching `CHANGELOG.md` section, commit, then tag:

```sh
git tag -a v0.2.0 -m "v0.2.0"
git push origin main --tags
```

The `Release` workflow builds the sdist plus wheels (Linux x86_64/aarch64
manylinux, macOS arm64/x86_64, Windows x86_64), attaches them to a GitHub
release, and publishes where credentials/variables allow.

## 2. PyPI (trusted publishing, no token)

One-time setup:

1. On PyPI, add a **pending trusted publisher** for the project
   `onenote-converter` with:
   - owner: `wsmontes`, repository: `one-note-converter`
   - workflow: `release.yml`, environment: `pypi`
2. In the GitHub repo, create an environment named `pypi`.
3. Set the repository variable `PYPI_PUBLISH=true`.

The tag `v0.1.0` was cut before a few documentation-only commits landed on
`main` (the `brew trust` step and the abi3 floor in the README). The `v0.1.0`
source tarball therefore carries the earlier text; `0.2.0` will include them.
Don't move the tag — that would change the Homebrew checksum and re-trigger the
blocked workflow for a text-only diff.

Until step 3 the publish job is skipped, so a tag never leaves a red run behind.
From then on, pushing a tag publishes automatically (with PEP 740 attestations).

Manual path, without Actions (works today):

```sh
cd python && maturin build --release --sdist --out ../dist
twine check ../dist/*                       # both artifacts must PASS
twine upload --username __token__ --password "$PYPI_API_TOKEN" ../dist/*
# or: maturin publish --username __token__ --password "$PYPI_API_TOKEN"
```

`pip install onenote-converter` only starts working after this first upload (or
after the trusted publisher above is in place).

## 3. crates.io

The crate is `one-note-converter` (the Python crate has `publish = false`).

One-time setup: add the repository secret `CARGO_REGISTRY_TOKEN` (a crates.io
API token) and create an environment named `crates-io`. The publish job is
skipped while the secret is missing.

Manual path (works today, needs a crates.io token):

```sh
cargo login "$CARGO_REGISTRY_TOKEN"   # or paste the token when prompted
cargo publish --locked
```

## 4. Homebrew tap

The formula lives in [`wsmontes/homebrew-onenote`](https://github.com/wsmontes/homebrew-onenote)
and is installed with `brew install wsmontes/onenote/onenote2md`.

The tap's scheduled workflow does the bump automatically **when Actions is
available**; while the account is billing-locked, update it by hand after each
release:

```sh
shasum -a 256 <(curl -sL https://github.com/wsmontes/one-note-converter/archive/refs/tags/v0.2.0.tar.gz)
# update url/sha256 in Formula/onenote2md.rb
brew audit --strict --new wsmontes/onenote/onenote2md
```

## 5. Checking a release

```sh
pip install --no-cache-dir onenote-converter==0.2.0
python -c "import onenote; print(onenote.__version__)"
brew update && brew upgrade onenote2md && onenote2md --help
cargo install one-note-converter --version 0.2.0
```
