# rfluence-convert

Markdown <-> Confluence ADF (Atlassian Document Format) conversion that round-trips: a page converted to markdown and back is the same page, and markdown converted to ADF and back is the same markdown. Content markdown can't express is kept in markers in the markdown.

This is the converter behind [rfluence](https://github.com/ckedwards/rfluence), the command-line tool for working on Confluence pages as markdown. It does no I/O. Its API follows rfluence's needs and may change between versions; most people want the [`rfluence`](https://crates.io/crates/rfluence) command instead.

Licensed under the MIT license.
