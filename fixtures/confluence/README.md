# Confluence reference pages

Real Confluence responses for a set of reference pages, one folder per page. The converter
must handle what Confluence *returns*, not just what rfluence sends (design.md, "What 'same
content' means"), so these are its test data. Tests only read these files; they never call
Confluence, so they keep working if the test site goes away.

## Current site

| | |
| --- | --- |
| Site | `https://tech-accounts11.atlassian.net` (a test account) |
| Space | [`rfluencete`](https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/) "rfluence-test-fixtures" (ID 753705) |
| Parent | the space homepage (page 753877) |

The pages were moved here from a personal space on 2026-10-04. A move keeps page IDs,
versions, history, attachments and editor-made content, so the fixtures stayed valid. It also
moves the page's children: `space-homepage`'s three template pages came along. Confluence
rewrote the space key in every page URL inside the moved bodies (without a new version), so
those links were re-captured; the saved earlier version of `adf-reference` still has the old
space key, and its test ignores space keys.

Page IDs in `design.md` (e.g. "verified on test page 98404") refer to this site; the table
below maps them to fixtures. To move the pages within the site, move them in Confluence (or
with `PUT /wiki/rest/api/content/{id}/move/append/{new parent id}`) and re-capture with
`--all --force`. To recreate them on another site, see [Restoring the pages](#restoring-the-pages).

## The pages

| Fixture | Page ID | Title | Made with | What it covers |
| --- | --- | --- | --- | --- |
| `adf-reference` | 458790 | rfluence ADF reference | API, then editor | Every common node type (see below), a section added by hand in the editor, and labels |
| `code-languages-and-widths` | 98404 | rfluence code language and width API test | API, then editor re-save | 197 code block language names; `breakout` and table width variants |
| `emoji` | 426008 | rfluence emoji API test | API, then editor re-save | Emoji nodes A–F (below) |
| `emoji-names` | 720904 | rfluence emoji names API test | API | Emoji nodes with names Confluence doesn't use (below): the browser renders all of them, so rfluence needs no Atlassian emoji data |
| `images-api` | 131074 | rfluence image API test | API | Images A–E: minimal node, explicit size/layout, two attachment versions, external image |
| `links-and-anchors` | 295349 | rfluence link API test | API | Page links (text, URL with title, smart link) and heading anchors (punctuation, Unicode, duplicates, extra spaces) |
| `mermaid-api` | 458755 | rfluence merfluence API test | API | Diagrams A (source only), B (source + default settings), C (changed source + stale cached SVGs: shows the old diagram) |
| `mermaid-editor` | 295341 | merfluence | Editor | A diagram inserted in the editor: the full merfluence node with cached SVGs and `embeddedMacroContext` |
| `mermaid-viewer-editor` | 1966084 | Mermaid Diagrams Viewer | Editor | Two Mermaid Diagrams Viewer macros inserted in the editor: one after a code block in an expand (paired automatically, `guestParams: ""`), one after a plain code block with the code block picked in its settings (`guestParams: {"index": 0}`, which counts all code blocks, so it shows the first diagram's source). The code blocks have no `language` |
| `tabs-and-synced-block` | 2392065 | rfluence tabs and synced block | Editor | Tabs (three tabs, the first holding a table), a synced block (the original), and two empty layouts |
| `synced-block-copies` | 1605660 | rfluence sync block destination | Editor | Two copies of synced blocks: one from `tabs-and-synced-block`, one from an unpublished draft (page 2195459, so it shows as unavailable) |
| `space-homepage` | 295257 | Software development | Confluence space template | A page made by people: layouts, expands, panels, images, smart links |

### `adf-reference`

Created through the API with code blocks (with/without language, backticks, inside a list),
tables (header column, numbered, column widths, merged cells, cell colour, image and nested
expand in cells, list and code in cells), nested lists, task and decision lists, all panel
types including a custom one, every text mark, alignment, indentation, quote, rule, heading
links on this page and another, inline / block / embed smart links, images (caption, alt text,
border, link), a PDF file card, emoji, status, mention, date, expand, TOC / excerpt / child
pages macros, and a three-column layout.

Below its "(editor) Added by hand" heading: content inserted in the editor for comparison: an
inline comment, a custom emoji (`:rfluence:`), statuses, mentions and dates (inline and in a
table), a resized table, a resized image with border, alt text and link, a pasted smart link,
a word-wrapped code block and a widened code block.

Its labels (in `page.json`) are the result of adding labels through the API: `two`, `words`,
`ünïcode`, `dash-ok`, `under_score`, `upper`, `comma`, `label`. They were first tested on a
separate page (524309, since trashed), where `UPPER` was stored as `upper`, `two words` split
into `two` + `words` and `comma,label` into `comma` + `label` (design.md, "Labels").

### `emoji`

| | Emoji node |
| --- | --- |
| A | `shortName` only (`:rotating_light:`) |
| B | `shortName`, `id`, `text` |
| C | a GitHub alias as `shortName` (`:+1:`; shows a question mark) |
| D | an unknown `shortName` (shows a question mark) |
| E | an emoji with a zero-width joiner, all attributes |
| F | `✅` and `:tada:` typed as plain text |

### Removed

  * `labels` (page 524309): merged into `adf-reference`; its body was a single line.
  * `markdown-paste` (page 426029): `architecture.md` pasted into the editor from a rendered
    preview. It tested Confluence's HTML paste, which rfluence never uses, and its content was
    covered by the other pages.

## Other test content on the site (not captured)

Folder trees used to verify the page-tree rules for `upload --config` (design.md, "Page
hierarchy", "Renames and deletions"). They aren't fixtures, weren't moved (they're still in
the personal space), and the restore script doesn't recreate them; the findings are recorded
in design.md.

  * [rfluence tree test](https://tech-accounts11.atlassian.net/wiki/spaces/~71202005eb3148b0e1450da60d67073e3cc131/folder/131110) (folder ID 131110, under the space homepage) has nested folders and pages for checking hierarchy: folder `api`, page `api` (same title as the folder), folder `rfluence tree page` (same title as a page), and page `rfluence tree page` (moved from `api` to the top folder).
  * [rfluence lifecycle test](https://tech-accounts11.atlassian.net/wiki/spaces/~71202005eb3148b0e1450da60d67073e3cc131/folder/262167) (folder ID 262167, under the space homepage) has the results of the rename/delete tests: folder `new-name` holding a page with an `rfluence` content property (moved there from the trashed folder `old-name`), and pages moved up to this folder when their parent page / folder was deleted. A folder `old-name` and page `lifecycle parent page` were recreated after their originals were trashed, to show trashed titles can be reused. The originals are in the space trash.

### `emoji-names`

Each line has an emoji node that a GitHub-based implementation might send: 1 Atlassian's name
(baseline), 2 a GitHub name Atlassian doesn't use, 3 an `id` with the variation selector where
Atlassian's has none, 4 both, 5 a base name with a skin-tone `id`, 6 an emoji missing from
Atlassian's catalog, 7 an empty name, 8 the characters as the name. All eight render as the
right emoji in the browser (checked by hand, 2026-10-04); the server renderer shows the image
only for 1 and the characters for the rest. Its `page.md` also shows the known limitation for
literal shortcode text: the label "Atlassian calls it :pencil:" fetches as "📝".

## Files in a fixture

| File | Contents |
| --- | --- |
| `adf.json` | The page body (ADF) as Confluence stores it, decoded and pretty-printed |
| `page.json` | The rest of the page response: `id`, `title`, `version`, `labels`, `parentId`, links (keys sorted) |
| `attachments.json` | The page's attachments, including each one's `fileId` (keys sorted) |
| `attachments/` | The attachment files, so the page can be recreated |
| `page.md` | The page as `rfluence fetch` writes it (reference output). Images point at `attachments/`, so it previews with them |
| `adf.v*-api-save.json` | An earlier version, saved by hand (see below) |

### Saved earlier versions

Three pages were created through the API and later saved in the Confluence editor. The
version from before the editor save was kept, because comparing the two shows what an editor
save rewrites (design.md, "What Confluence rewrites on save"):

| Fixture | Earlier version | Later version (`adf.json`) |
| --- | --- | --- |
| `adf-reference` | `adf.v2-api-save.json`: API only | v5: the "(editor)" section added; the API-made part above it rewritten by the save |
| `code-languages-and-widths` | `adf.v2-api-save.json`: API only | v3: re-saved with " XX" typed into the first paragraph |
| `emoji` | `adf.v1-api-save.json`: API only | v2: re-saved with a paragraph "XXX" added |

These pairs can't be recreated: a restored page is API-made. The capture script won't
overwrite these fixtures without `--force`.

## How tests use them

All in `crates/rfluence-convert/tests/`:

  * `fetch.rs`: every page converted to markdown must match its `page.md`, and the editor-save
    tests: each saved earlier version must give the same markdown as the later one, apart
    from the edits listed above. After an intended change to fetch output, run
    `RFLUENCE_UPDATE_FIXTURES=1 cargo test` and review the `page.md` diffs.
  * `roundtrip.rs`: for every page, fetch -> upload -> fetch gives identical markdown.

## Capturing

```shell
scripts/capture-fixtures.sh <fixture>...            # re-capture (page ID from page.json)
scripts/capture-fixtures.sh --all                   # all except those with saved versions
scripts/capture-fixtures.sh --new <fixture> <id>    # add a page
```

Re-capturing replaces a fixture with the page as it is now. If the page was edited, update its
`page.md` with `RFLUENCE_UPDATE_FIXTURES=1 cargo test` and review the diff.

### Adding a reference page

1. Create the page on the site (through the API or the editor; say which in the table above).
2. `scripts/capture-fixtures.sh --new <fixture> <page id>`, with a lowercase-with-dashes name.
3. Add it to this README: what it covers and how it was made.
4. Run `RFLUENCE_UPDATE_FIXTURES=1 cargo test` to write its `page.md`, and review it.

## Restoring the pages

If the test site or space goes away, or the pages should live elsewhere, recreate them from
the fixtures:

```shell
scripts/restore-reference-pages.py --space <KEY> [--parent <id>] [--title-suffix " (copy)"] [--dry-run] [fixture...]
```

Credentials for the target site come from `.env` / the environment. It creates every page
with its labels and attachment files, then writes the bodies with image IDs remapped to the
new attachments and links between the reference pages rewritten to the new site, space and
page IDs. `--dry-run` reports what would happen and what won't carry over. The fixtures aren't
changed. Afterwards, update [Current site](#current-site) and the page IDs in the table.

What doesn't carry over (the script lists each case it finds):

  * **Editor-made content becomes API-made.** The restored pages hold the same content, but
    were saved through the API, so the editor's quirks are gone from them. The fixtures keep
    the original responses, so don't re-capture editor-made pages (`mermaid-editor`,
    `space-homepage`, the saved-version pairs) from restored copies.
  * **Inline comments**: their anchors are dropped; the comment threads stay on the old site.
  * **Custom emoji** (`:rfluence:` on `adf-reference`): site-specific; upload one again by hand.
  * **Mentions** keep their account IDs and show correctly only for accounts on the new site.
  * **merfluence diagrams** need the merfluence app installed on the new site.
  * **Attachment history**: only each attachment's current version is captured (`images-api`
    tested references to an older version of `square.svg`).
  * **Links to pages that aren't reference pages** (e.g. from `space-homepage` to its template
    pages) still point at the old site.

Tested on 2026-10-04 by restoring `images-api`, `links-and-anchors` and `labels` into the same
space with a title suffix: the restored pages fetched to the same markdown as the fixtures
apart from the rewritten links, and kept all labels. (The test copies were moved to the trash.)
