# Notebook fixture

`notebook/` is a table of contents plus one section group and a nested group, so the
Python tests can exercise the tree without a large sample. The `.one` files are
copies of `tests/fixtures/testOneNote2016.one` placed under the names the
`.onetoc2` entries expect; that file is Apache Tika test data, Apache-2.0 (see
`tests/fixtures/README.md`).
