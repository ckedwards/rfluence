# Reading pages

## Search

```shell
rfluence search "rate limit"                         # free text, pages only
rfluence search "rate limit" --space ENG --space OPS  # in any of these spaces
rfluence search "deploy" --label runbook              # with this label (repeat for several: all of them)
rfluence search "deploy" --limit 25                   # more results (default 10)
rfluence search --cql 'title ~ "onboarding" and lastmodified > now("-30d")'
```

Each result shows the page ID, title, space, last update, labels, URL and an excerpt. `--json` gives the same as JSON.

## Fetch a page

A page can be named by its ID, its URL (including short `/x/…` links), or `SPACE:Title`:

```shell
rfluence fetch 123456
rfluence fetch https://example.atlassian.net/wiki/spaces/ENG/pages/123456/Release+process
rfluence fetch "ENG:Release process"
```

There are two forms:

- **The round-trip form** (the default) keeps everything: settings and Confluence-only content are in `<!-- rf: … -->` comments and ```` ```adf ```` blocks, so the file can be uploaded back. Use it to [edit](editing.md) pages.
- **`--simplified`** is for reading: shorter, with Confluence-only things as plain text (statuses as `[DONE]`, mentions as `@Name`, macros as `[Table of contents]`). It can't be uploaded.

```shell
rfluence fetch 123456 --simplified
```

## Long pages

```shell
rfluence fetch 123456 --max-chars 20000          # cut at about 20,000 characters, listing the sections
rfluence fetch 123456 --section "Rollback"       # one section, by heading text or #anchor
```

## Save to a file

```shell
rfluence fetch 123456 -o docs/release.md
```

This also downloads the page's images to `docs/release.assets/`, and turns links to pages you have as files in the same project into relative links. See [Editing and creating pages](editing.md).

`--json` prints the page's metadata and markdown as JSON, for scripts.
