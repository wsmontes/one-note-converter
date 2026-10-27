# one-note-converter

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`onenote2md` converts Microsoft OneNote content to Markdown, in Rust, with the same parser exposed to
Python.

|Input|Output|
|---|---|
|`.one` section|Markdown with headings, lists, tables, pictures, attachments, tags, ink markers|
|`.onetoc2` notebook|a folder tree: one file per section plus an `index.md` per level|
|`.onepkg` package (Microsoft Cabinet)|the notebook or sections inside it|

Two on-disk layouts are read: plain revision stores (`[MS-ONESTORE]`,
`{109ADD3F-911B-49F5-A5D0-1791EDC8AED8}`) and sections wrapped in the `[MS-FSSHTTPB]` package store
(`{638DE92F-A6D4-4BC1-9A36-B3FC2511A5B7}`), which is how OneNote for Windows and OneDrive deliver
files. Packaged-store support is marked experimental in the output because page bodies can be
incomplete; the warning is printed, counted in `--report`, and turns into a failure under `--strict`.

## Install

```sh
# Python library — published on PyPI (CPython >= 3.9, abi3 wheels for
# Linux, macOS and Windows)
pip install onenote-converter        # https://pypi.org/project/onenote-converter/

# CLI, macOS or Linux — builds from the tagged source
brew trust wsmontes/onenote          # one-time, for third-party taps
brew install wsmontes/onenote/onenote2md

# CLI plus library plus Python module, from source
git clone https://github.com/wsmontes/one-note-converter.git
cd one-note-converter && cargo build --release   # target/release/onenote2md
cd python && python -m venv .venv && . .venv/bin/activate
pip install -r requirements-dev.txt && maturin develop --release

# or install an artifact from the release page (each release ships five wheels
# plus a portable sdist that builds anywhere Rust is available)
pip install https://github.com/wsmontes/one-note-converter/releases/download/v0.1.2/onenote_converter-0.1.2.tar.gz

# CLI from crates.io (same binary, built by cargo)
cargo install one-note-converter
```

## Usage

```sh
cargo build --release

# structure report: object spaces, roots, object types, invariants, pages
./target/release/onenote2md inspect Section.one

# drill down while reverse engineering a file
./target/release/onenote2md inspect Section.one --decls                  # every object + blob range
./target/release/onenote2md inspect Section.one --object 2:14            # one property set
./target/release/onenote2md inspect Section.one --blob 0x138f8           # raw property set offset
./target/release/onenote2md inspect Section.one --locate 0x14000         # which object owns an offset
./target/release/onenote2md inspect Section.one --find-text "Sunday"     # which object carries text

# header dump (1024 bytes, field by field)
./target/release/onenote2md header Section.one

# Markdown to stdout, or to a file with pictures extracted next to it
./target/release/onenote2md convert Section.one -o Notes.md
./target/release/onenote2md convert Section.one -o out/Notes.md --assets out/images
# Preserve page-level YAML metadata by writing one Markdown file per page.
# With --split-pages, -o names a DIRECTORY containing index.md and assets/.
./target/release/onenote2md convert Section.one --split-pages --front-matter -o out/pages
./target/release/onenote2md notebook "Open Notebook.onetoc2" --split-pages --front-matter -o out/notebook-pages


# a whole notebook: every section listed in the table of contents
./target/release/onenote2md notebook "Open Notebook.onetoc2" -o out/notebook
```

`convert` writes pictures to `<section>_assets` next to the output file by default, naming them after
the OneNote image name plus a short GUID so duplicates such as `Untitled picture.png` stay distinct.

`notebook` follows the table of contents **given on the command line** (a folder with several
`.onetoc2` files does not make it guess); sections are written as `<name>.md` next to their assets,
and each level gets an `index.md` listing them in notebook order. In OneNote a section group is a
subfolder with its own `.onetoc2`, so groups become subfolders with their own index. A section that
the table of contents lists but the folder does not contain is reported and makes the command exit
non-zero, so a batch run cannot mistake a partial notebook for a complete one.

`--front-matter` emits YAML metadata for each page. In the default single-file-per-section
layout, YAML for subsequent pages is not recognized as file-level front matter by most Markdown
processors. Use `--split-pages` to produce one Markdown document per page (plus `index.md`),
with valid YAML for each page, in `convert`, `notebook` and `onepkg` alike. Dates come from the page node's own `LastModifiedTime` and the file's creation stamps, so a
page that never recorded one simply omits the key. `--report` writes a JSON summary
(`files[].{path,kind,pages,assets,failures,warnings}` plus `totals`), and `--strict` exits non-zero
when that summary holds any warning — packaged stores report their experimental status that way.

`onepkg` reads the cabinet directly: uncompressed folders and MSZIP (the deflate history carries
across blocks, so each block is inflated with the previous 32 KiB as dictionary). Quantum and LZX
are rejected with a message instead of guessed. Package paths are sanitised — absolute paths and
`..` are refused — before anything is written.

## Python bindings

`python/` is a PyO3 extension module over this crate, so the parser is shared, not
re-implemented:

```python
import onenote

section = onenote.read_section("Section.one")
section.pages[0].title
section.markdown(front_matter=True)
section.write("out/Section.md", "out/img")
section.write_pages("out/pages", front_matter=True)  # one file per page

notebook = onenote.read_notebook("Open Notebook.onetoc2")
notebook.all_sections()
notebook.write("out/notebook")

onenote.read_package("Open Notebook.onepkg").write("out/package")
```

Build it with `maturin develop --release` inside `python/` (wheel:
`maturin build --release`); see `python/README.md` for the full API.

## What the parser reads

The layout is documented by Microsoft as [MS-ONESTORE] (revision store) and [MS-ONE] (property ids,
object types, root roles). The chain implemented here:

```
header (1024 B)
└── root file node list                     -> one FileNode per object space
    ├── ObjectSpaceManifestListReferenceFND  -> object space manifest list
    │   └── RevisionManifestListReferenceFND -> revision manifests
    │       └── ObjectGroupListReferenceFND  -> object groups
    │           └── ObjectDeclaration*FND    -> BlobRef -> ObjectSpaceObjectPropSet
    └── FileDataStoreListReferenceFND        -> FileDataStoreObject (embedded pictures)
```

Details that the implementation had to get right, each verified against real files:

- `FileNode.Size` includes the 4-byte node header, and nodes are not aligned.
- File node lists are bounded by the transaction log: only the first `cTransactionsInLog`
  sentinel-terminated transactions are committed, and each `TransactionEntry` declares the active
  number of nodes for one list. Nodes past that length belong to an incomplete transaction and are
  dropped (`ONENOTE_TRACE_NODES=1` reports it; no file in the test corpus needs it).
- The hashed chunk list (`fcrHashedChunkList`) holds `HashedChunkDescriptor2FND` nodes: a `BlobRef`
  plus its MD5. `header` and `inspect` report the entry count and hashes.
- A file node list spans fragments; only the non-final fragments carry `ChunkTerminatorFND`, so the
  final fragment ends in zero padding instead.
- `PropertySet.cProperties` is **2 bytes**. One level down, `prtArrayOfPropertyValues` has a
  4-byte element count followed by a **single** `prid` describing the type shared by every element
  (not one prid per element, which shifts the whole payload).
- The high bits of a `PropertyID` carry its type, which determines the size and location of the
  value; strings are length-prefixed UTF-16, `TextExtendedAscii` is single-byte Windows-1252.
- `RootObjectReference3FND.oidRoot` is an **ExtendedGUID**, not a CompactID: it resolves to
  `(gid_table_index << 8) | n`. Root role 1 is the content root, role 2 metadata, role 4 revision
  metadata.
- Pictures live in `FileDataStoreObject` blobs framed by two fixed GUIDs.
- Rich text embeds RTF-ish field codes starting with a noncharacter in `U+FDD0..=U+FDEF`; the
  `HYPERLINK "…"` instruction is dropped and the field result kept.

## Which content becomes Markdown

Each page is a page object space, identified by its role-1 root (a `jcidPageManifestNode`), and
pages are ordered by the section's page series. Page content is walked through every child
collection the format uses (`ElementChildNodes`, `ContentChildNodes`, `StructureElementChildNodes`),
so a node's type does not have to be guessed to reach its children.

Pages are rendered as `# Title`, paragraphs (indented by `OutlineElementChildLevel`), bullets, tables
whose cells may hold text and pictures, images, and embedded files as links (the attachment bytes are
extracted next to the pictures). Whole-paragraph bold/italic/underline come from
the element's `ParagraphStyle` object. A `jcidImageNode`'s `RichEditTextUnicode` — the text OneNote
keeps for the picture — becomes the image's alt text.

The output reflects the **current** state: superseded revisions and version-history/conflict spaces
are not rendered. When the current declaration of an object carries no text at all, the text of a
superseded declaration of the same object is used instead (this is what recovers the alt text of
embedded XPS/PDF previews).

## Validated on

44 files from two independent corpora (Tika test documents, and the samples of
[onenote.rs](https://github.com/msiemens/onenote.rs) — Windows exports, Joplin, ink, German):

| Corpus | Files | Revision store | Converted | Legacy container |
| --- | --- | --- | --- | --- |
| Apache Tika test documents | 10 | 9 | 7 | 1 |
| onenote.rs samples | 34 | 7 | 7 | 25, plus 2 Git-LFS pointer stubs |

Across the 14 files that converted: 24 pages, 189 text blocks and 71 assets extracted (pictures and
attachments), **0** objects skipped, **0** partially decoded property sets, **0** malformed property
set padding, and no panics. The only dangling object reference in the corpus is inside a deliberately
corrupted Tika fuzz fixture.

Those baseline figures were gathered before FSSHTTPB support was implemented. The current parser
also recognizes packaged stores, but this path remains experimental: page bodies may be incomplete.
The original standalone TOC and deliberately corrupted test fixture are not convertible sections.
Additional independent end-to-end fixtures are needed to measure current packaged-store fidelity.

## Verified against

Apache Tika's `OneNoteParserTest` is used as an independent oracle, since its assertions were
written against the same fixtures by a different implementation:

| Tika assertion (fixture) | Result |
| --- | --- |
| `testOneNote2016`: "So good", "This is one note 2016" | exact match, asserted by `tests/onestore.rs` |
| `testOneNote2`: two page GUIDs | 2 pages |
| `testOneNote2`: "Section1HeaderTitle", "Section1TextArea1", "Section1TextArea2", "wow this is neat" | present |
| `test-tika-3970-dupetext`: "Sunday morning" appears once | exactly 1 |
| `test-tika-3970-dupetext`: one page GUID | 1 page |
| `test-tika-4303-Chinese-notes`: `中文标题` | present |
| `testOneNote1`: two pages | 2 pages |
| `assertNoJunk`: no font names, no mis-decoded UTF-16 | no "Calibri", no C1 controls, no stray CJK in Latin fixtures |

Known difference: Tika merges text from *all* revision contexts, so it also reports text that exists
only in another context's version of the same object (for example "neat info about totally killin it
bro" in `testOneNote2`, where the current page holds "wow this is neat" for that node). This
converter renders the current revision and does not merge divergent edits.

## Known limits

- Empty paragraphs that exist only for vertical spacing are not reproduced.
- Note tags appear as a `[label]` prefix; the tag's own colour/shape is not reproduced.
- Ink strokes are not rendered: the page shows a `> ink drawing not rendered` marker where they were.
- Front matter is opt-in (`--front-matter`, `markdown(front_matter=True)`) and best effort: a page
  that recorded no date simply omits the key.
- `Charset` other than the default code page (0, Windows-1252) is not honoured; East Asian
  single-byte text would need the matching code page.
- Packaged stores (`[MS-FSSHTTPB]`) are experimental: page bodies can be incomplete and the run
  reports it as a warning.
- Encrypted sections are detected and reported; their content is not decrypted.
- Pictures that a file references without a payload (web clippings, external caches) are named in the
  Markdown, listed by `missing_images()`, and counted as warnings in conversion statistics and
  `--report` (and errors under `--strict`); there are no bytes to write.
- Quantum and LZX cabinet compression is rejected; stored and MSZIP folders are read.

## Provenance and license

This crate is MIT licensed (see `LICENSE`). All formats were implemented from the published
specifications — [MS-ONESTORE] (revision store, property ids) and [MS-FSSHTTPB] (package store) —
and validated against real files plus the Apache Tika test assertions.

`onenote-parser` (msiemens, MPL-2.0) was consulted as an independent implementation to cross-check
observed behaviour while reverse engineering the package store. The code here is an independent
implementation written from the specifications, not a port: the bit layouts and element orderings are
transcribed from [MS-FSSHTTPB]/[MS-ONESTORE] and documented in `src/fsshttpb.rs`, so the module does
not depend on that project.

## Tests

`./scripts/check.sh` runs the same commands the CI workflow runs, in the same
order: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --release -p one-note-converter`, `maturin build` and install of the
wheel plus `pytest -q`, `maturin build --sdist` with `twine check`, installing that sdist
into a clean venv, and `actionlint` when it is installed. It works on a bare
checkout: without `python/.venv` it creates a throwaway one from
`python/requirements-dev.txt`.

```sh
./scripts/check.sh
```



```sh
cargo test
```

The integration test parses `tests/fixtures/testOneNote2016.one` (from the Apache Tika test corpus,
Apache-2.0 — see `tests/fixtures/README.md`) and checks the header, that every property set consumes
its blob modulo `0..=7` bytes of padding, that no object reference dangles, and the exact title,
body and Markdown produced.
