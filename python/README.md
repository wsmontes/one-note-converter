# onenote — Python bindings

`pip install onenote-converter` (the distribution is `onenote-converter`, the module is `onenote`).

Python API over the same Rust parser the `onenote2md` CLI uses. Nothing is
re-implemented here: a `.one` section, a `.onetoc2` notebook and a `.onepkg`
package are parsed by the crate in the parent directory.

## Install

Requires Rust (stable) only when building.

```bash
# development: build and install into the active virtualenv
python -m venv .venv && . .venv/bin/activate
pip install maturin
maturin develop --release   # or: maturin build --release + pip install target/wheels/*.whl

# a wheel for distribution (abi3: one wheel for CPython ≥ 3.8)
maturin build --release      # → target/wheels/onenote-<ver>-cp38-abi3-<platform>.whl
pip install target/wheels/onenote-*.whl
```

## API

```python
import onenote

section = onenote.read_section("Section.one")        # path or bytes
section.page_count                                   # int
page = section.pages[0]
page.title, page.level, page.created, page.modified  # ISO-8601 UTC strings
page.authors                                         # list[str]
page.images                                          # list[Image]

section.markdown(front_matter=True)                  # str, pictures as bare names
section.images()                                     # list[Image] with .filename/.data

stats = section.write("out/Section.md", "out/img", front_matter=True)
stats.pages, stats.assets, stats.failures
```

`Image.data` is `bytes`. Names are unique and deduplicated per section **and are
the same names `markdown()` links to**, so the in-memory flow is safe:

```python
md = section.markdown()
for image in section.images():
    Path(image.filename).write_bytes(image.data)   # every delivered `![](name)` resolves
```

`Page.images` uses the same names, filtered to the pictures that page references.
Links are percent-encoded; `Image.filename` is the raw name.

Some links have no payload in the file — OneNote web clippings and external
caches reference a picture without embedding it. Those are listed by
`section.missing_images()` (raw names), so callers can tell "delivered" from
"referenced only": `links ⊆ {i.filename for i in section.images()} ∪ set(section.missing_images())`.

```python
notebook = onenote.read_notebook("Open Notebook.onetoc2")
notebook.name, notebook.toc
notebook.sections          # [NotebookSection(name, path)]
notebook.groups            # [Notebook] for section groups
notebook.errors            # entries whose file/folder is missing
notebook.all_sections()    # every section, depth first

stats = notebook.write("out/notebook", front_matter=True)
# out/notebook/<Section>.md + <Section>_assets/, one index.md per level
```

The tree reflects what conversion does: a folder with its own `.onetoc2` is a
group even when the parent index does not list it, and `.onetoc2` dotfiles (the
OneNote Recycle Bin sentinel) are ignored.

```python
package = onenote.read_package("Open Notebook.onepkg")
package.entries            # file names inside the cabinet
package.sections()         # parse every .one entry
package.extract_to("dir")  # write the entries and return their paths
stats = package.write("out/package", front_matter=True)
```

`Package.write` follows the CLI: when the cabinet carries a table of contents it
converts a notebook tree, otherwise each section on its own. MSZIP and stored
(no compression) folders are supported; Quantum/LZX raise `ValueError`.

## Errors

Malformed files raise `ValueError` with the parser's message; unreadable paths
raise `OSError`. `write` never raises for a missing picture link — it reports the
paths in `stats.failures` and still writes the Markdown.

## Tests

```bash
python -m pytest tests -q
```

The fixtures come from `../tests/fixtures` plus `tests/data/notebook`, a small
notebook tree (a table of contents, a group and a nested group). The package
tests build their cabinet in Python, independently of the Rust reader.

## Limitations

Same as the CLI: `[MS-ONESTORE]` revision stores are fully read, `[MS-FSSHTTPB]`
package stores are marked experimental, encrypted sections are reported but not
decrypted, and cabinet folders compressed with Quantum or LZX are rejected.
