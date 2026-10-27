# Test fixtures

`testOneNote2016.one` is taken from the Apache Tika test corpus:

- Source: <https://github.com/apache/tika/blob/main/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/testOneNote2016.one>
- License: Apache License 2.0 (<http://www.apache.org/licenses/LICENSE-2.0>)
- Content: a single page titled "So good" with one line of body text.

The file is used unmodified as a parser fixture; it is not part of the
converter's own source license.

`test-tika-3970-dupetext.one` (2.6 MB) is taken from the same Apache Tika test
corpus:

- Source: <https://github.com/apache/tika/blob/main/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-microsoft-module/src/test/resources/test-documents/test-tika-3970-dupetext.one>
- License: Apache License 2.0 (<http://www.apache.org/licenses/LICENSE-2.0>)

It is committed because the duplicate-object regression only exists there: the
file carries heaps of redundant objects and the test asserts that no paragraph is
emitted twice.

Both binaries are unmodified parser fixtures; they are not part of the
converter's own source license.
