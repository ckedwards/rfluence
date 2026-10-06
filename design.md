# rfluence

Create a new Rust monorepo (Cargo workspace) with the following targets.

## Core objective

The core objective is to create a loop to support bidirectional uploads and downloads from Confluence. MD files created by AI can be uploaded seamlessly to Confluence and can then be downloaded and be the same content. Round trips are an important design and essential goal. To support this, the merfluence Confluence app has been installed on the test site and is a requirement for this to work.

### What "same content" means

Byte-for-byte equality is not achievable (list markers, emphasis style, table alignment and whitespace are not preserved by Confluence), so round trips are defined against a canonical markdown form:

  * All markdown is passed through a normalizer (e.g. comrak's CommonMark renderer) before comparison.
  * Round-trip rule: `normalize(md) == fetch(upload(md))`.
  * Fixed point: a second upload/fetch cycle produces no changes.
  * Confluence rewrites content on save (adds `localId` attributes, reorders/normalizes nodes). The converter must handle what Confluence *returns*, not just what was sent. Recorded test responses must capture real Confluence responses for this reason.

The normalizer is comrak's CommonMark renderer with these settings (verified against the corpus in `fixtures/markdown` by the normalizer tests in `rfluence-convert`: fixed point, unchanged AST, `rf:` comments still attached):

  * Extensions: strikethrough, table, autolink, tasklist, footnotes, alerts, front matter (`---`). Not comrak's shortcodes extension; emoji are handled by the converter, see "Emoji".
  * Emoji are written as characters: known shortcodes (`:tada:`, GitHub's `:+1:`) become characters, and emoji characters are written the way fetch writes them; see "Emoji".
  * `render.prefer_fenced = true`. Without it, a fence with no info string is written as an indented code block, and two adjacent indented blocks merge into one on the next parse (a lost code block).
  * Trailing whitespace is stripped from every line outside code and HTML blocks after rendering. comrak leaves it on lines like `> ` and blank lines inside list items; hard breaks are rendered as `\`, so it never carries meaning outside those blocks.
  * comrak inserts `<!-- end list -->` between adjacent lists so they don't merge when re-parsed. The converter treats it as a list separator and never uploads it as content.
  * comrak only lists a marker like this when a list is followed by a code block or another list; it guards against an indented code block joining the list, which can't happen with fenced code, but the marker is harmless.
  * Inline formatting is flattened into ADF-style marks and re-nested in a fixed order (link, strong, em, strike, then the HTML marks below), so `**[a](u)**` and `[**a**](u)` normalize to the same text. Fetch uses the same order.
  * Soft line breaks become spaces (ADF paragraphs have none, so a wrapped line comes back as one line).
  * Lists are made tight and ordered lists use `.` markers (ADF has no loose/tight distinction or marker style).
  * ```` ```adf ```` fences are rewritten to fetch's form (see "Content Confluence has that markdown doesn't").
  * Markdown with a close Confluence equivalent is rewritten into it (`<kbd>` -> inline code, ...), exactly as upload does; see "Checking markdown".
  * `rf:` comments (and fence settings) are rewritten in one spacing, key format and key order (`<!--rf:width=1 layout=center-->` -> `<!-- rf: layout=center width=1 -->`), the order fetch writes them in.
  * comrak bug: an HTML block's text is written raw, bypassing the line prefixes of enclosing quotes and list items, so its trailing newline ends a blockquote (`> <!-- c -->`, `>`, `> text` came out as two quotes). Workaround: HTML block text loses its trailing newline before rendering, and comrak starts the next line itself, with the prefix. Multi-line HTML blocks inside quotes or list items are still affected; worth reporting upstream.
  * Expected restyling, all with an unchanged AST: `-` bullets, ATX headings, `*` / `**` emphasis, `\` hard breaks, reference links inlined, entities decoded, compact table delimiter rows, longer fences when the content contains backticks, footnote text on an indented line under `[^name]:`.

### What Confluence rewrites on save

Verified on test page 458790, which was created through the API with every common node type; the stored ADF was diffed against what was sent. Beyond the image and link rewrites described in their own sections:

  * **Code block languages are renamed**: `bash` is stored as `shell`. A ```` ```bash ```` fence would come back as ```` ```shell ````, so the converter needs an alias table like the one for emoji (see Open questions).
  * **Marks are reordered** into a canonical order (`link`, `em`, `strong` for a bold italic link). Compare marks as a set, and render them in a fixed order on fetch.
  * **Adjacent text nodes with the same marks are merged**, and empty text nodes are dropped. The converter must not depend on how text is split.
  * **Defaults are filled in**: `orderedList` gets `order: 1`, `table` gets `layout: "default"`, every table cell gets `colspan: 1` / `rowspan: 1`, `status` gets `style: "bold"`. As with images, defaults must not produce `rf:` comments.
  * **Links get `__confluenceMetadata`** (`linkType` `self` / `page`, `anchorName`, `contentTitle`, `versionAtSave`), as do `inlineCard` and `blockCard` page links. Fetch ignores it; upload doesn't send it.
  * **Mentions**: `text` is replaced with the user's current display name (`@me` -> `@Chris Edwards`), and an empty `accessLevel` is dropped.
  * **Macros** get `macroId` and `macroParams._parentId` (the page ID). The Children Display macro was upgraded (`schemaVersion` 1 -> 2, title `Child pages`) and its `localId` dropped. For ```` ```adf ```` passthrough, this means macro JSON fetched from one page carries that page's ID; upload should strip `macroId` and `_parentId` so a block copied to another page doesn't point back at the original.
  * Stored as sent: table `colwidth`, `isNumberColumnEnabled`, merged cells, cell `background`, `nestedExpand`, `caption`, image `alt`, `border` and `link` marks on media, `mediaGroup`, panel types including custom colour and icon, `alignment` and `indentation` marks, `subsup`, `textColor`, `backgroundColor`, `decisionList`, nested `taskList`, `blockCard`, `embedCard`, `date`, `layoutSection` with three columns, `bodiedExtension` (excerpt), TOC macro.

An **editor save rewrites the whole page**, not just the part a person changed. Verified on page 458790 by editing only its last section in the editor; the API-created blocks above it were rewritten too:

  * Every node gets a `localId` (12 hex characters), and nodes without attrs get `attrs: {}`.
  * Attributes the API save added are removed again: `__fileName` / `__fileMimeType` / `__fileSize` on images, `__confluenceMetadata` on links and cards, `macroParams._parentId` on macros.
  * Tables get `width: 760`; code blocks, expands and layouts get a `breakout` mark (`mode: "wide"`, `width: 760`); embed cards get `originalWidth` / `originalHeight`.
  * Marks are reordered again, into a different order from the API save (`strong`, `em`, `link` instead of `link`, `em`, `strong`). Neither order is canonical.
  * Empty paragraphs added in the editor are kept (`{"type": "paragraph", "attrs": {}}`). Markdown has no empty paragraph, so these are dropped on fetch.

So the converter must treat `localId`, `__*` attributes, default `breakout` marks and table widths, `__confluenceMetadata`, `_parentId`, empty `attrs`, mark order and text-node splits as noise: none of them may produce a markdown change. Otherwise any human edit in Confluence would show up as a diff across the whole page. `breakout` and table `width` are open (see Open questions).

Editor-inserted nodes, compared with the same nodes created through the API (page 458790, last section):

  * Status: the editor writes `style: "mixedCase"` (the API save fills in `bold`), and custom colours are stored as hex (`color: "#FFF0B3"`) as well as named colours. Status nodes are preserved as ```` ```adf ```` blocks, so this only matters for fixtures.
  * Mention: `{"id": "<accountId>", "text": "@<display name>"}`, no `accessLevel`.
  * Date: `{"timestamp": "<ms since epoch>"}`, same as the API.
  * Inline comment: an `annotation` mark on the text, `{"annotationType": "inlineComment", "id": "<uuid>"}`. See "Inline comments" for how upload keeps it.
  * Image resized in the editor: `mediaSingle` with `layout: "align-end"`, `width: 653`, `widthType: "pixel"`; `media` keeps its natural `width` / `height` (128); `border` and `link` marks on `media` as with the API.
  * Resized table columns: `colwidth` as floats (`[193.0]`), table `width: 760.0`.
  * The code block language picker has no `bash`, only `shell`, so `bash` -> `shell` is Confluence's naming, not an API quirk.
  * Code block widened in the editor: `breakout` mark with `mode: "wide"` and `width: 4000` (the editor default is `width: 760`).
  * Code block word wrap is a viewing toggle, not page content: turning it on stores nothing in the ADF or the page's content properties, and it is gone after a page refresh or in a private window. The converter ignores it; there is nothing to round-trip.

### Content Confluence has that markdown doesn't

Pages edited by humans in Confluence will contain things markdown can't express (Jira macros, status lozenges, @mentions, layouts, other app macros). `fetch` must not drop these, or the next `upload` will delete them. Unsupported nodes are preserved as opaque fenced blocks containing the raw ADF JSON (e.g. ```` ```adf ````), which `upload` passes back unchanged.

Forms (all are kept by the normalizer and passed back unchanged by upload):

  * **Block nodes** (decisions, file cards, macros, custom panels, tables that don't fit a GFM table, ...): a ```` ```adf ```` fence holding one node as compact one-line JSON: `type` first, attribute keys sorted, save noise removed (`adf::strip_noise`). One line rather than pretty-printed, so a large node doesn't flood an LLM's context.
  * **Inline nodes** (status, mention, date, inline macros, ...): `<span data-adf='{json}'>visible text</span>`. The visible text (status text, `@display name`, `YYYY-MM-DD`) is there for people and LLMs; upload ignores it and uses the JSON. `'` in the JSON is written as `\u0027`.
  * **Marks without markdown syntax**: inline HTML that upload maps back: `<u>`, `<sub>`, `<sup>`, `<span style="color: #ff5630">`, `<span style="background-color: #fedec8">`.
  * A paragraph or heading with marks rfluence can't express falls back to an ```` ```adf ```` fence, so nothing is dropped.

### Markdown features LLMs commonly produce

These should map to native Confluence elements where possible:

  * Mermaid code blocks -> merfluence macro
  * GitHub alerts (`> [!NOTE]`, `> [!WARNING]`, ...) -> Confluence info/note/warning panels
  * Task lists (`- [ ]`) -> Confluence task lists
  * Code fences with language tags -> code blocks with language (see "Code block languages and widths")
  * Emoji (characters, and shortcodes like `:rotating_light:`) -> Confluence `emoji` nodes (see "Emoji")
  * Tables (including pipes inside inline code), nested lists mixed with code blocks, footnotes, inline HTML

### Element mapping

Decided while implementing `rfluence-convert`; settings follow "Confluence-only settings in markdown".

  * **Panels <-> GitHub alerts**, matched by colour: `info` <-> `[!NOTE]`, `note` <-> `[!IMPORTANT]`, `success` <-> `[!TIP]`, `warning` <-> `[!WARNING]`, `error` <-> `[!CAUTION]`. Custom panels (colour / icon) are ```` ```adf ```` fences.
  * **Tables** are GFM tables when GFM can express them: every cell a single paragraph (line breaks as `<br>`), no merged cells or cell colours, header cells exactly the first row and/or first column, and each column's alignment and width consistent. Column alignment <-> `alignment` marks on every cell paragraph in the column (`center`, `end`; `start` doesn't exist, see normalizer). `rf:` keys after the table: `layout`, `width` (non-default for the layout), `colwidths=193,566`, `numbered`, `no-header-row`, `header-column`.
  * **Other tables are HTML tables with markdown in their cells** (lists, code, images, expands, ... in cells; merged and coloured cells). Fetch writes, and normalize produces, one canonical form: runs of tags on their own lines, each cell's content as markdown between blank lines (the blank lines are what make markdown inside HTML parse, as on GitHub):

    ```markdown
    <table>
    <tr>
    <th colspan="2">

    Cell content as **markdown**

    </th>
    </tr>
    <tr>
    <td rowspan="2" style="background-color: #e3fcef">

    - a list in a cell

    </td>
    </tr>
    </table>
    ```

    `<th>` / `<td>` <-> `tableHeader` / `tableCell`; `colspan`, `rowspan`, `style="background-color: ..."` <-> the cell attributes. Column widths and the other table settings use the same `rf:` comment as GFM tables; when cells' widths don't agree per column, each cell gets `data-colwidth="120,200"`. `<details>` in a cell is a nested expand. Tables an HTML table can't express (unknown cell or table attributes) stay ```` ```adf ````.
  * **HTML tables as LLMs write them** are accepted and normalized to the canonical form: everything on one line (`<table><tr><td>text</td>...`), `<thead>` / `<tbody>` / `<tfoot>`, omitted closing tags, and HTML lists (`<ul><li>`) or `<p>` in cells, which become markdown. An HTML table GFM can express is normalized to a GFM table, as fetch would write it. A GFM table with an HTML list in a cell becomes an HTML table (warning).
  * **Paragraph / heading settings**: an `rf:` comment at the end of the line: `align=center|end`, `indent=<level>`.
  * **Images**: `rf:` keys `layout` (not `align-start`), `width` (when not the natural width), `width-type` (when not `pixel`), `border`, `border-color`, `caption` (plain-text captions; others make the image an ```` ```adf ```` fence). A linked image is `[![alt](src)](href)`. An image from another page's collection stays ```` ```adf ````.
  * **Cards**: `<url>` alone in a paragraph with `<!-- rf: card=block -->` or `<!-- rf: card=embed layout=center width=100 -->`; with the target page's title, `[Title](url)<!-- rf: card=block -->` (see "Smart links" under "Links").
  * **Mermaid** fence settings: `theme`, `mermaidVersion`, `useMaxWidth=false` (non-default values only).
  * **Task lists**: a nested task list follows its parent `taskItem` in ADF and goes inside the item in markdown. Task and decision `localId`s are generated deterministically on upload (`000000000001`, ...), so the same markdown gives the same ADF.
  * **Links**: a link whose text is its URL is written as `<url>` by comrak, so it uploads as an `inlineCard`. A plain text link in Confluence whose text equals its URL therefore becomes a smart link after a round trip.
  * **Expands <-> `<details>`**: an expand is written as `<details><summary>Title</summary>`, a blank line, its content as markdown, a blank line, and `</details>`. The opening tag and summary are on one line (a one-line HTML block also avoids the comrak bug above). On upload, `<details>` at the top level or in a layout column is an `expand`, and inside an expand a `nestedExpand`; anywhere else is an error (Confluence allows expands only there). Other ways of writing `<details>` (summary on its own line, everything in one HTML block, `<details open>`) are rewritten to this form. An expand with a non-default width stays ```` ```adf ````.
  * **Layouts <-> column markers**: a layout section is written as its columns' content as markdown, between `rf:` markers:

    ```markdown
    <!-- rf: columns=50,50 -->

    First column's markdown.

    <!-- rf: column -->

    Second column's markdown.

    <!-- rf: end-columns -->
    ```

    `columns=` lists the column widths (percent), and the marker also takes `breakout=` / `width=` like code blocks. Layouts are only allowed at the top level. Markdown renderers don't show the comments, so the columns read as consecutive sections. (`layout=` is a different setting: the Confluence `layout` attribute of tables, images and cards.)
  * **Line breaks in table cells** are written as `<br>` (a GFM cell is one line), so such tables stay GFM tables.
  * **Empty paragraphs** and **trailing hard breaks** in a paragraph are dropped (markdown can't express them).

### Checking markdown

Markdown that Confluence can't store exactly is reported by `rfluence check` and by upload (`rfluence_convert::check`, `Upload::diagnostics`), with a line number.

**Warnings**: there is a close Confluence equivalent, and upload uses it. Normalize makes the same change, so `normalize(md) == fetch(upload(md))` still holds (`fixtures/markdown/approximated.md`):

  * `<kbd>`, `<code>`, `<samp>`, `<tt>` -> inline code
  * `<b>` / `<strong>`, `<i>` / `<em>`, `<s>` / `<del>` / `<strike>` -> markdown formatting; `<ins>` -> `<u>`
  * `<br>` -> a markdown line break (kept as `<br>` in table cells)
  * Other inline HTML tags (`<abbr>`, `<span class=...>`, ...) and inline HTML comments: dropped, text kept
  * Image titles (`![a](b "title")`): dropped
  * Alert titles (`> [!WARNING] Title`): written as a bold first line in the panel
  * `<details open>`: open state dropped (expands always start closed)
  * A GFM table with an HTML list in a cell: written as an HTML table
  * HTML table cell attributes and styles other than `colspan`, `rowspan`, `background-color` and `data-colwidth`; table captions: dropped
  * Unknown `rf:` settings, and `rf:` comments not attached to anything: ignored
  * Local images whose file is missing (`rfluence check` only; upload reuses an attachment with that name if the page has one)

**Errors**: can't be represented in Confluence. Upload refuses unless forced, and then uploads an approximation (`fixtures/markdown/unsupported.md`):

  * Footnotes (`[^1]`): uploaded as literal text and a paragraph
  * Raw HTML blocks other than `<details>` (`<div>`, block HTML comments): uploaded as `html` code blocks
  * Inline images in a sentence (Confluence images are blocks): uploaded as links
  * Quotes nested in quotes, alerts in quotes (ADF quotes can't hold them): flattened
  * Content in a task other than its text and nested task lists: dropped
  * `<details>` where Confluence doesn't allow expands, or without a matching `</details>`; layout markers that don't form a layout (missing `end-columns`, column count not matching `columns=`, a layout not at the top level)
  * A table inside a table cell (ADF tables can't nest); `<table>` without `</table>`; content between HTML table rows outside any cell
  * Invalid ```` ```adf ```` JSON or `data-adf` spans

### Inline comments

An inline comment is an `annotation` mark on the commented text, `{"annotationType": "inlineComment", "id": "<uuid>"}` (observed on page 458790). The thread itself lives on the server; the mark is its anchor. If an upload sends the text without the mark, the thread is detached from the page.

Inline comments are kept out of the markdown and re-anchored on upload:

  * Fetch drops `annotation` marks. The markdown stays clean: no IDs for an LLM to read past or break, and annotations never affect `normalize(md) == fetch(upload(md))`.
  * Upload already GETs the remote page for the version check; that call also returns the body. A pure function in `rfluence-convert` takes the remote ADF and the new ADF and puts each annotation back on the same text in the new ADF, matching by the annotated text and its occurrence index (the nth match in the document), then by surrounding text if the occurrence index no longer matches. This mirrors Confluence's own anchoring: the v2 inline comments API locates a comment by `textSelection` + `textSelectionMatchIndex`.
  * An annotation whose text was changed or deleted can't be re-anchored, and its comment is detached, the same as when someone deletes commented text in the Confluence editor. Ambiguous matches are not guessed. Upload and `--dry-run` report both, e.g. `2 inline comments will be detached: "…"`.
  * An annotation can cover part of a text node, several text nodes with different marks, or text in several paragraphs (one mark per text node, same `id`). The re-anchoring works on the plain text of each block, then splits text nodes as needed.
  * Implemented in `rfluence_convert::annotations::reanchor`: each comment's annotated runs (adjacent text nodes joined) are matched in the new body's text, where blocks are separated and other inline nodes (emoji, mentions, ...) are placeholders, so a match can't span them. The nth occurrence is used if it still exists, else the only occurrence; anything else is reported, not guessed.
  * Verified on a temporary test page (since trashed): comments added with the v2 inline comments API (`textSelection`, `textSelectionMatchIndex`) are `annotation` marks in the body, and adding them doesn't create a page version. After an upload that changed the text around one comment and deleted another's text, the first comment's mark was on its text in the new version; the second was reported as detached, and the comments API still lists it as `open` (only the anchor is gone).

### Mermaid diagrams (merfluence)

Merfluence is a Forge app. A diagram is a single `extension` node (not `bodiedExtension`), and the Mermaid source is stored verbatim in the page's ADF, so diagrams can round-trip. Observed on the test site (page 295341):

```json
{
  "type": "extension",
  "attrs": {
    "layout": "default",
    "extensionType": "com.atlassian.ecosystem",
    "extensionKey": "5321c3d1-955d-42ac-9f09-d4d6f0802224/04b85365-6260-47d9-9f03-8f42e258aab7/static/mermaid-diagram",
    "text": "Merfluence",
    "parameters": {
      "layout": "extension",
      "guestParams": {
        "source": "flowchart TD\n    A[Start] --> B{Is it?}\n    ...",
        "theme": "auto",
        "mermaidVersion": "auto",
        "useMaxWidth": true,
        "renderedVersion": "11.16.1",
        "cacheV": 3,
        "svgLight": "<svg ...>  (~19 KB)",
        "svgDark": "<svg ...>  (~19 KB)"
      },
      "forgeEnvironment": "PRODUCTION",
      "embeddedMacroContext": { "accountId": "...", "cloudId": "...", "extensionData": { "content": { "id": "295341", "version": 1 }, "space": { ... } } },
      "localId": "...",
      "extensionId": "ari:cloud:ecosystem::extension/5321c3d1-.../04b85365-.../static/mermaid-diagram",
      "extensionTitle": "Merfluence"
    },
    "localId": "..."
  }
}
```

  * Identify merfluence nodes by `extensionType == "com.atlassian.ecosystem"` and an `extensionKey` ending in `/static/mermaid-diagram`. The key is `<app id>/<environment id>/static/<module>`.
  * Fetch: `guestParams.source` becomes a ```` ```mermaid ```` fence. Never output `svgLight` / `svgDark`; they are ~19 KB each per diagram and would flood an LLM's context.
  * Non-default `theme`, `mermaidVersion` and `useMaxWidth` values go in the fence info string, e.g. ```` ```mermaid theme=dark ```` (see "Confluence-only settings in markdown").
  * `source` has no trailing newline; markdown fence content does. Normalize this.
  * Upload: building the node requires the app and environment IDs. The environment ID isn't public, so both are discovered from the site (see "Mermaid diagrams on sites without merfluence" > "Finding which Mermaid app a site has"); only the app ID, from merfluence's manifest, is built in.
  * Upload a minimal node: `guestParams.source` plus the IDs, with no cached SVGs and no `embeddedMacroContext` / `localId`. Verified on test page 458755: merfluence renders this correctly, and Confluence stores it as sent. Viewing the page does not write cached SVGs back.
  * **Never send cached SVGs with a changed `source`.** Verified: when `source` changes but the old `svgLight` / `svgDark` are kept, merfluence shows the old diagram. Simplest rule: always strip `svgLight`, `svgDark`, `renderedVersion` and `cacheV` on upload. Keeping them for unchanged diagrams is a possible later optimization.
  * `embeddedMacroContext` holds page-specific data (page ID, version, space, account). Don't send it on upload.

### Mermaid diagrams on sites without merfluence

A ```` ```mermaid ```` fence is uploaded as the best form the site supports:

  1. merfluence is installed: a merfluence diagram (above).
  2. Otherwise, [Mermaid Diagrams Viewer](https://marketplace.atlassian.com/apps/1232887/mermaid-diagrams-viewer) (Atlassian Labs, [source](https://github.com/atlassian-labs/mermaid-diagrams-viewer)) is installed: the source in a collapsed expand, followed by the viewer's macro, which draws the diagram from that code block.
  3. Neither: a code block with language `mermaid`. No warning: it's the honest form, it round-trips, and a viewer installed later can use it.

How Mermaid Diagrams Viewer works (from its source, and the editor-made page in fixture `mermaid-viewer-editor`):

  * The macro (`extension`, key `mermaid-diagram`) holds no diagram. It reads the page's ADF and shows a code block's text. With no setting (`guestParams` empty), the nth viewer macro on the page shows the nth Mermaid code block: language `mermaid` (the editor can't set it, but the API can), or text Mermaid recognises. With a code block picked in its settings (`guestParams: {"index": n}`), it shows the nth code block of **any** language, which breaks as soon as a code block is added above it. rfluence never sets `index`.
  * The viewer needs `parameters.localId` (it throws without one) to find its own position.

Upload form (verified on a temporary test page, since trashed: two diagrams with a Python code block between them rendered correctly, and the expands start collapsed):

```json
{"type": "expand", "attrs": {"title": "Mermaid source"}, "content": [
  {"type": "codeBlock", "attrs": {"language": "mermaid"}, "content": [{"type": "text", "text": "flowchart TD\n  A --> B"}]}]},
{"type": "extension", "attrs": {
  "extensionType": "com.atlassian.ecosystem",
  "extensionKey": "<app id>/<environment id>/static/mermaid-diagram",
  "text": "Mermaid diagram", "layout": "default", "localId": "<uuid>",
  "parameters": {"localId": "<same uuid>", "extensionId": "ari:cloud:ecosystem::extension/<app id>/<environment id>/static/mermaid-diagram", "extensionTitle": "Mermaid diagram"}}}
```

  * No `guestParams`, `forgeEnvironment` or `embeddedMacroContext` are needed. Confluence stores the nodes as sent, including `language: "mermaid"`.
  * The expand keeps the source out of the way on the page; the viewer still finds the code block inside it.
  * Fence settings that only merfluence has (`theme=`, ...) are dropped, with a warning.

Fetch:

  * Exactly that pair (an expand titled `Mermaid source` holding only a `mermaid` code block, directly followed by a viewer macro without `index`) becomes a ```` ```mermaid ```` fence, so it round-trips.
  * Anything else stays lossless: the expand as `<details>`, the code block as a fence, the macro as ```` ```adf ````. That includes viewer diagrams made in the editor (another expand title, no expand, no language, or an `index` setting), since rfluence can't upload them back in the same form.
  * `--simplified` (for reading only) writes every viewer macro as the ```` ```mermaid ```` fence of the code block it shows, and leaves out the code block it was drawn from.

Choosing the app on upload (implemented):

  * A merfluence diagram already on the page decides (its app, which may be a fork). Otherwise, only when the body has a Mermaid fence, the site's macros are listed (GraphQL, below; two requests in parallel with the page fetch): merfluence wins over the viewer. If the lookup fails, diagrams are uploaded as code blocks, with a warning.
  * A viewer diagram needs an expand, which is only allowed at the top level and in layout columns; elsewhere (lists, tables, quotes, ...) a Mermaid fence stays a code block.
  * Each viewer macro gets its own `localId` (`00000000-0000-4000-8000-<n>`, numbered within the page).
  * The viewer counts every macro whose key ends in `mermaid-diagram`, merfluence's included, when pairing macros with code blocks. Upload uses one app for the whole page, so the two don't mix on pages it writes.
  * An app that forks merfluence or the viewer has another app ID, so the lookup doesn't find it; a per-site setting could cover that later (see Future considerations).
  * Verified: the live upload test (merfluence installed) uploads a Mermaid fence as a merfluence diagram and fetches it back unchanged. The viewer form was verified separately (above), since the test site has both apps.

Finding which Mermaid app a site has (verified on the test site):

  * App IDs are in each app's Forge manifest, so they're stable unless the app is forked: merfluence `5321c3d1-955d-42ac-9f09-d4d6f0802224` ([manifest](https://github.com/edlopez000/merfluence/blob/main/manifest.yml)), Mermaid Diagrams Viewer `23392b90-4271-4239-98ca-a3e96c663cbb` ([manifest](https://github.com/atlassian-labs/mermaid-diagrams-viewer/blob/main/app/manifest.yml)). Both apps' macro key is `mermaid-diagram`.
  * The environment ID in `extensionKey` isn't in the manifest (Forge assigns it on deploy), so it has to be discovered.
  * The GraphQL gateway lists the macros installed on a site, with app and environment IDs, in one call (about 0.2 s). It's the query the editor uses for its macro menu; it works with an API token (basic auth). Needs the site's cloud ID, from `GET /_edge/tenant_info` (no auth). The operation must be named, and the macro extension type is `xen:macro` (`confluence:macro` and `macro` return nothing):

    ```graphql
    query rfluence_macros($contextIds: [ID!]!, $type: String!) {
      extensionContexts(contextIds: $contextIds) {
        extensionsByType(type: $type) { id appId environmentId environmentType key properties }
      }
    }
    ```

    with `{"contextIds": ["ari:cloud:confluence::site/<cloud id>"], "type": "xen:macro"}`, sent to `POST /gateway/api/graphql`. On the test site it returns merfluence: `appId` `5321c3d1-…`, `environmentId` `04b85365-…`, `environmentType` `PRODUCTION`, `key` `mermaid-diagram`, `id` `ari:cloud:ecosystem::extension/<app>/<env>/static/mermaid-diagram`. After installing Mermaid Diagrams Viewer it also returns the viewer: `appId` `23392b90-4271-4239-98ca-a3e96c663cbb` (matching its manifest), `environmentId` `63d4d207-ac2f-4273-865c-0240d37f044a`, `PRODUCTION`, `key` `mermaid-diagram`, title "Mermaid diagram". Both apps' macros have the same key, so they're told apart by app ID. Not yet checked: whether an account without site admin rights sees the same list.

### Tabs and synced blocks

Observed on editor-made pages (fixtures `tabs-and-synced-block`, page 2392065, and `synced-block-copies`, page 1605660):

  * **Tabs** are a `multiBodiedExtension` (`extensionType: "com.atlassian.confluence.native"`, `extensionKey: "native-tabs"`). `parameters.tabs` lists each tab's `{id, title}` (`id` is 6 random characters), plus `applyToAll` and `extensionTitle: "Tabs"`. Each tab's content is an `extensionFrame` child, in the same order.
  * **A synced block** (the original) is a `bodiedSyncBlock` with `attrs.resourceId` (a UUID) and its content inline.
  * **A copy** of it, on any page, is a `syncBlock` with no content: `attrs.resourceId` is `confluence-page/<source page id>/<resourceId>`. Showing it means reading the source page. The source can be a draft (page 2195459, readable with `get-draft=true` by its author); the editor then shows "Synced content will display when the page is published". Not checked: what a reader without access to the source page sees.

What the API accepts (verified on temporary pages, since trashed):

  * Tabs created with the API (the editor's node without `localId`s) render correctly.
  * A `syncBlock` copy created with the API, pointing at an editor-made synced block, shows that block's content. Confluence stores it with `localId: ""` added.
  * A `bodiedSyncBlock` with a new `resourceId` is stored and its content shows on its own page, but it isn't a synced block: a copy pointing at it shows "We're unable to display this synced block as it's not available on this site". Synced blocks are registered outside the page body when the editor creates them, so rfluence can reference existing ones but not create new ones.
  * Editing an existing synced block's content through the API (page 2392065 version 2, `resourceId` unchanged; restored in version 3) changes only the source page: it showed the new text, while both copies (on page 1605660 and a temporary page) kept showing the old text. The copies still worked, so the block stayed registered. The content copies show is kept outside the page body and only the editor updates it, so a synced block's content must be treated as read-only by rfluence: uploading an edit would silently make the source page and its copies disagree.

Markdown forms (fixtures `tabs-and-synced-block` and `synced-block-copies` show them):

  * **Tabs** round-trip, with markers like layouts:

    ```markdown
    <!-- rf: tabs -->

    <!-- rf: tab title=Install -->

    Run `make install`.

    <!-- rf: tab title="Getting started" -->

    - one

    <!-- rf: end-tabs -->
    ```

    Only the editor's form is written this way (default settings, one `extensionFrame` per tab); anything else stays ```` ```adf ````. Upload allows tabs at the top level and in layout columns, treats a tab's content like a layout column's, gives an empty tab an empty paragraph, and numbers the tab IDs (`rf0001`, `rf0002`, ...; the editor uses 6 random characters). Verified on a temporary page uploaded by `rfluence upload` (since trashed): tabs with these IDs render and switch. `--simplified` writes each tab's title as a bold line, then its content.
  * **Synced blocks** are read-only, between markers that carry their ID; the `read-only` flag is there for LLMs reading the markdown:

    ```markdown
    <!-- rf: synced-block id=b7229247-1d30-4ce9-84dc-accb011d4a6b read-only -->

    This is a sync block

    <!-- rf: end-synced-block -->
    ```

    * The page's own synced block has `id=` only. Upload sends the block as Confluence has it (from the page it already reads), never the markdown.
    * A copy also has `page=<source page ID>`. Fetch reads the source pages' bodies in one request (`GET /wiki/api/v2/pages?id=...&body-format=atlas_doc_format`, which returns only published pages) and writes the source block's content between the markers; a copy whose source can't be read (no access, deleted, a draft) gets the `unavailable` flag and no content. Upload sends a `syncBlock` reference. Adding a copy of an existing synced block works: write the markers with `page=` and `id=`, with nothing between them.
    * Upload refuses (exit 2, before anything is sent) when the content between the markers differs from Confluence's (compared like `fetch -o`'s check for local edits, so link destinations and image folders don't count), when a block without `page=` isn't on the page (synced blocks can't be created through the API), and when a copy has content but its source can't be read to check it. Leaving nothing between the markers always means "as it is". The message says how to fix it: edit the block in Confluence's editor, then fetch again.
    * Removing the page's own synced block from the markdown removes it from the page; upload warns that its copies may stop showing its content (not verified what they show).
    * `rfluence check` (offline) checks only the markers: `id=` present, markers balanced.
    * `--simplified` writes the content without markers, and `[Synced block from page <id>: not available]` for an unreadable copy.
  * The rfluence skill should tell LLMs that synced block content is read-only, and that `<!-- rf: ... -->` markers must be kept.

Verified end to end on the same temporary page: a copy added from empty markers (`page=` and `id=` only) shows the source block's content, and fetching the page gives the tabs and the copy's content back.

An outage worth knowing about when live tests fail strangely: on 2026-10-06, for a few hours, creating pages on the test site failed in the browser and the API alike. A new page got an ID, then every request for it returned "Page not found" (404), while reading and updating existing pages worked. The page tree kept entries for those pages, and the browser left untitled drafts; `DELETE /wiki/api/v2/pages/{id}` (with `?draft=true` for drafts) removed them even though they couldn't be read. Atlassian's status page showed no incident.

Before this, fetch kept tabs and synced blocks as ```` ```adf ```` blocks (lossless, but unreadable); `--simplified` ran the tabs' content together without their titles, and wrote nothing for a copy, so a page of copies read as empty.

### Emoji

An emoji in ADF is an inline node: `{"type": "emoji", "attrs": {"shortName": ":thumbsup:", "id": "1f44d", "text": "👍"}}`. Standard emoji have the hex codepoint(s) as `id` (`-`-joined, keeping zero-width joiners: `1f62e-200d-1f4a8`).

Names (checked against the catalogs on 2026-10-03):

  * Atlassian's standard catalog (`GET <site>/gateway/api/emoji/standard`, 1890 emoji) gives each emoji exactly one `shortName`, with no aliases.
  * GitHub's shortcodes, which LLMs write, often differ for the same emoji: of 1913 Unicode GitHub names, 1200 match Atlassian's name, 451 are the same emoji under another name (`:+1:` -> `:thumbsup:`, `:memo:` -> `:pencil:`, `:stop_sign:` -> `:octagonal_sign:`, country flags `:albania:` -> `:flag_al:`), and the rest didn't match by `id`, mostly because GitHub drops zero-width joiners from codepoints. 23 GitHub names (`:octocat:`, ...) are images with no Unicode emoji.

What Confluence does (verified on test page 426008):

  * Emoji nodes are stored exactly as sent. Missing attrs are not filled in, and invalid `shortName`s (`:+1:`, `:not_an_emoji:`) are accepted.
  * In the browser, a valid `shortName` renders even without `id` / `text`, but an invalid one (`:+1:`, `:not_an_emoji:`) shows a question mark. So GitHub aliases must be translated, not passed through.
  * The server renderer (`body-format=view`) is stricter: without an `id` it shows a blue-star placeholder, even for a valid `shortName`. Upload therefore always sends `shortName` + `id` + `text`, which render in both.
  * Unicode emoji and `:name:` in plain text are left as text, never converted to emoji nodes.
  * An editor save doesn't repair emoji nodes: after page 426008 was edited and re-published in the editor, the nodes only gained a `localId`. Missing `id` / `text` and invalid `shortName`s stay as they were, so upload must get all three attrs right, and fetch must handle `shortName`-only nodes written by other tools (the catalog lookup does).

What Confluence needs in an emoji node (verified on test page 720904, created through the API and viewed in the browser):

  * The browser renders an emoji from its `id` / `text`, whatever its `shortName`: a GitHub name Atlassian doesn't use (`:memo:` for Atlassian's `:pencil:`), a base name with a skin-tone `id`, an empty `shortName`, the characters as `shortName`, an `id` including the variation selector where Atlassian's omits it, and an emoji missing from Atlassian's catalog all render as the right emoji.
  * The server renderer (`body-format=view`) shows Confluence's emoji image only for Atlassian's own `shortName`s, and the `text` characters otherwise, so those still show the right emoji.
  * Atlassian's `id`s are the codepoints of the emoji's characters (all 3659 catalog entries), sometimes with and sometimes without the variation selector.

So rfluence doesn't need Atlassian's emoji data. Emoji data (characters, GitHub shortcodes, skin tones) comes from the `emojis` crate.

Converter rules. Markdown holds emoji as Unicode characters (`🎉`, `👍🏽`): they read the same in any renderer and in an LLM's context. GitHub shortcodes are accepted as input.

  * Recognising emoji characters in text: longest match first (so skin tones, flags and joined sequences like `😮‍💨` match whole), with or without variation selectors. Characters that are plain text by default (`©`, `™`, `✔`, `⛹`; anything below U+1F000 that isn't Unicode `Emoji_Presentation`) are emoji only when followed by the variation selector U+FE0F, so prose like `© 2026` is left alone.
  * Shortcode syntax: `:name:` in text (never in code), where `name` is letters, digits, `_`, `+` or `-`, not directly preceded or followed by a letter or digit (so `1:100:3` is not an emoji). Known names are GitHub's (`:tada:`, `:+1:`); Atlassian-only names (`:flag_nz:`, `:thumbsup::skin-tone-3:`) aren't recognised; write the characters instead.
  * Upload: emoji characters and known shortcodes become emoji nodes with `text` = the fully-qualified characters, `id` = their codepoints (`1f44d-1f3fd`), and `shortName` = the GitHub shortcode (the base emoji's, for a skin-tone variant), or the characters if there is none. Other shortcodes go to the custom emoji lookup (below), and stay text if that finds nothing.
  * Fetch: an emoji node is written as its characters, taken from `text` if that is an emoji, else decoded from `id`, else looked up from `shortName` (repairs nodes written by other tools with only a name, like `:+1:`). Otherwise it is a custom emoji, written as `:shortName:` (below). Characters are written fully qualified (`❤️`, with the variation selector), so upload recognises them again.
  * Normalize: known shortcodes become characters (`:tada:` -> `🎉`, `:+1:` -> `👍`), and emoji characters are written as fetch writes them, so `normalize(md) == fetch(upload(md))` holds.
  * Known limitation: plain text in Confluence that looks like a known shortcode (`:tada:` typed without being converted, as in case F on test page 426008) becomes an emoji after fetch -> upload. Telling it apart would need an escape in markdown (`\:tada:`), which comrak doesn't preserve; an earlier implementation worked around that and was dropped as too fragile for a rare case (the editor converts `:tada:` to an emoji node as you type).
  * Emoji characters typed as text in Confluence (`✅` in case F) become emoji nodes after fetch -> upload. They look the same, but the node uses Confluence's emoji images instead of the system font.

Custom (site-uploaded) emoji. Observed on page 458790 after uploading a custom emoji `rfluence` through the editor:

  * The node is `{"shortName": ":rfluence:", "id": "8c4f3c94-1ade-4ce8-8b3a-0fdc390d2f04", "text": ":rfluence:"}`. The `id` is a UUID, which is all hex and dashes, so it can't be told apart from a codepoint sequence by its shape; the `text`-vs-`id` comparison above classifies it correctly (`text` is the shortcode, not the encoded characters).
  * Confluence refuses a custom emoji named like a standard one (uploading one named `tada` fails with "emoji name already exists").
  * Fetch writes them as `:<shortName>:` (they have no characters), using the rules above.
  * Upload resolves a `:name:` that isn't a known shortcode against the site emoji API (one extra call, only when such a name is present). A match becomes an emoji node with the site emoji's `shortName`, `id`, and `text` set to the shortName, as the editor writes it.
  * Known limitations, because fetch doesn't call the site emoji API (it must stay one API call) and normalize is pure:
    * Plain text matching a custom emoji's name (e.g. `:partyparrot:` typed as text) becomes the emoji after fetch -> upload, as for standard shortcodes.
    * Known shortcodes win over site emoji. A custom emoji named like a GitHub shortcode Atlassian doesn't use (e.g. `:memo:`) would be uploaded as the standard emoji. Clashes with Atlassian's own names can't happen (Confluence refuses them).
  * The site emoji API response also contains a media access token, so never store it in fixtures.

### Images and attachments

How images appear in ADF (observed on the test site, e.g. page 295257):

  * Attached image: a `mediaSingle` node (with `layout`, `width`, `widthType`) wrapping a `media` node with `type: "file"`, `id` and `collection: "contentId-<page id>"`. The `media.id` is **not** the attachment ID (`att...`); it matches the attachment's `fileId` in the v2 attachments API (`/wiki/api/v2/pages/{id}/attachments`). This gives image -> attachment -> filename -> download link.
  * External image: a `media` node with `type: "external"`, `url` and `alt`. Maps directly to `![alt](https://...)`.
  * Pages can have attachments that aren't referenced in the body (e.g. space header images). `upload` must never delete attachments it doesn't recognise.

What Confluence does on save (verified on test page 131074; all variants render correctly):

  * Missing image settings are filled in: a bare `mediaSingle` is stored as `layout: "align-start"` (left-aligned, not centred), `width` = the image's natural width, `widthType: "pixel"`. The `media` node gets `width` / `height`.
  * Explicitly sent settings (`layout`, `width`, `widthType`) are kept as sent.
  * Uploading a new version of an attachment gives it a **new `fileId`**. A `media` node that references an older version's `fileId` is rewritten to the current version's `fileId` on save, so `upload` can always use the latest `fileId` and never needs to track versions.
  * API-created images also get `__fileName`, `__fileMimeType` and `__fileSize` attributes. Editor-inserted images (page 295257) don't have them, so they can't be relied on.
  * External images render, and are stored with `layout: "align-start"` added to the `mediaSingle`.
  * Uploading attachments (with `minorEdit=true`) does not bump the page version.

Fetch:

  * Images are always written as relative paths next to the markdown file: `![roadmap](page.assets/roadmap.svg)`.
  * Files are only downloaded when output goes to a file (`rfluence fetch -o page.md` creates `page.assets/`). Printing to stdout doesn't download anything.
  * Filenames come from `__fileName` when present. Otherwise one extra call to the attachments API maps `fileId` -> filename (only for pages that have images).

Upload:

  * A local image file is uploaded as an attachment named after the file (attachment names are unique per page). If an attachment with that name already exists, a new version is uploaded only if the content changed. The `media` node then points at the attachment's `fileId`.
  * If the local file is missing but the page already has an attachment with that name, the existing attachment is reused. This makes "fetch to stdout, edit, upload" safe without downloading images.
  * A page must exist before files can be attached to it, so creating a new page with images is: create the page, upload attachments, update the body.
  * Attachments are uploaded with the v1 API (`POST /wiki/rest/api/content/{id}/child/attachment` for new files, `.../child/attachment/{attachment id}/data` for new versions; multipart, header `X-Atlassian-Token: no-check`). The response includes `extensions.fileId`, which goes into the `media` node.
  * A minimal node works: `mediaSingle` > `media` with `type: "file"`, `id` (the `fileId`) and `collection: "contentId-<page id>"`. No `width` / `height` or `__` attributes are needed.

Scope: images only for now. Other attachments (PDFs etc., shown as file cards in a `mediaGroup`) are kept as ```` ```adf ```` fences, so a fetch/upload cycle doesn't turn file cards into links; see Future considerations.

### Code block languages and widths

Languages (verified on test page 98404, one code block per name for 197 names LLMs commonly put on fences, including aliases and mixed case):

  * Confluence lowercases the language, then renames exactly these aliases: `bash` -> `shell`, `js` -> `javascript`, `py` -> `python`, `cpp` -> `c++`, `c#` -> `csharp`, `vb` -> `visualbasic`, `text` -> `plaintext`. `Bash` and `BASH` become `shell` too.
  * Every other name is stored lowercased but otherwise as sent, including names Confluence doesn't highlight (`golang`, `sh`, `none`, `mermaid`). An empty language (`""`) is removed.
  * The editor's language picker uses the renamed forms (it offers `shell`, not `bash`). An editor save renames nothing further: after page 98404 was re-saved in the editor, every language was unchanged.

Converter rules:

  * The language is the first word of the fence info string; the rest holds settings (`mermaid theme=dark`, `shell width=4000`).
  * `rfluence-convert` bundles the alias table above. Upload sends the lowercased, de-aliased name. Normalize rewrites the fence language the same way (```` ```Bash ```` -> ```` ```shell ````, ```` ```js ```` -> ```` ```javascript ````), so `normalize(md) == fetch(upload(md))` holds. Fetch writes the stored language as-is.
  * Names outside the table pass through. If a live round-trip test finds another rename, it goes into the table.

Widths (`breakout` mark on code blocks, expands and layouts; `width` / `layout` on tables). Verified on page 98404 (API) and 458790 (editor):

  * The API stores them exactly as sent: `breakout` with `mode: "wide"` or `"full-width"`, with or without `width`; table `width` and `layout` (`default`, `wide`, `full-width`, `align-start`). A table sent without `layout` gets `layout: "default"`.
  * An editor save fills in a width for each mode on blocks nobody touched (page 98404 re-saved in the editor):
    * code block / expand with no `breakout` mark -> `{mode: "wide", width: 760}`; it looks the same as no mark (checked in the browser);
    * `breakout` `{mode: "full-width"}` without a width -> `width: 1800` (code blocks and layouts);
    * `breakout` `{mode: "wide"}` without a width -> `width: 1011` (possibly depends on the editor's window size, so `rfluence` never sends this form);
    * table with no `width` -> `760` for `layout: "default"`, `960` for `"wide"`, `1800` for `"full-width"`; a table without `layout` gets `"default"`.
  * So defaults are per mode: `wide` -> 760, `full-width` -> 1800 for `breakout`; `default` -> 760, `wide` -> 960, `full-width` -> 1800 for tables. A missing width and the mode's default width are the same setting.
  * A width someone chose in the editor: `breakout` `{mode: "wide", width: 4000}` for a widened code block.

Converter rules:

  * Fetch writes nothing for defaults (no mark, or `wide` at 760). Otherwise code blocks get fence settings: `breakout=full-width` when the mode is full width, and `width=<n>` when the width is present and isn't the mode's default. Examples: ```` ```shell width=4000 ````, ```` ```shell breakout=full-width ````. `breakout` `{mode: "wide"}` without a width (only seen from the API) is written as `breakout=wide`.
  * Tables get the same rule in an `rf:` comment after the table: `layout=<layout>` when not `default`, `width=<n>` when present and not the layout's default.
  * Upload sends no `breakout` mark / table `width` for defaults, the mode without a width when the width is the mode's default, and the width otherwise. Either way, an editor save that fills in the default width doesn't change the markdown.
  * Expands with a non-default `breakout` are written as ```` ```adf ```` blocks; layout `breakout` goes in the `columns=` marker (`<!-- rf: columns=50,50 breakout=full-width -->`). Fetch removes default `breakout` marks (and other editor-save noise, see "What Confluence rewrites on save") from ```` ```adf ```` blocks, so an editor save doesn't change them.
  * Code block word wrap isn't stored (see "What Confluence rewrites on save"), so there is nothing to round-trip.

### Confluence-only settings in markdown

Some Confluence settings have no markdown syntax. To keep round trips independent of the remote page, they are stored in the markdown file itself:

  * **Code blocks (including Mermaid):** in the fence info string, e.g. ```` ```mermaid theme=dark ````.
  * **Everything else (images, and later table column widths, captions, ...):** an `rf:` HTML comment directly after the element, on the same line:

    ```markdown
    ![roadmap](page.assets/roadmap.svg)<!-- rf: layout=center width=1070 -->
    ```

  * **Tables:** a table has no line to share, so its `rf:` comment goes on the line directly after the last row. comrak parses it as an HTML block that is the table's next sibling (a block-level HTML comment ends a GFM table); normalizing inserts a blank line between them but keeps it the next sibling. Verified by the normalizer tests (`fixtures/markdown/images-and-links.md`).

    ```markdown
    | Quarter | Revenue |
    | --- | --- |
    | Q1 | 10 |
    <!-- rf: width=1200 -->
    ```

Rules:

  * Only non-default values are written. Defaults are what Confluence fills in when a setting is omitted, so a round trip without a comment is stable. For images that's left-aligned (`align-start`) at natural width with `widthType=pixel`; such an image has no comment.
  * The comment only applies when it's immediately adjacent to the element: on the same line for inline elements (comrak parses it as an inline HTML node following the image or link), or as the very next block for tables.
  * `key=value` pairs, not JSON: shorter and less likely to be broken by an LLM editing around it.
  * If the comment is missing (e.g. an LLM dropped it), Confluence defaults are used. The content still uploads; only the setting is lost.
  * HTML comments are invisible in rendered markdown, so the file still reads cleanly.

### Links

How links appear in ADF (observed on the test site):

  * Text link: a `link` mark on text with an absolute URL, e.g. `{"href": "https://<site>/wiki/spaces/SD/pages/295297"}`.
  * Smart link: an `inlineCard` node with only a URL; Confluence displays the target page's title and icon (e.g. page 295257 links to its templates this way).
  * Same-page heading link: `{"href": "#On-this-page"}`. Confluence anchors keep the heading's case and replace spaces with hyphens; GitHub-style anchors are lowercased with punctuation removed (`#on-this-page`).
  * Embeds (YouTube, Loom, ...): `embedCard` nodes with `url`, `layout` and `width`.

Markdown <-> ADF mapping:

  * `[text](url)` <-> `link` mark
  * `<url>` (autolink), or `[Title](url)<!-- rf: card=inline -->` <-> `inlineCard` (see "Smart links" below)
  * `<url>` alone on a line plus an `rf:` comment <-> `embedCard` / `blockCard`, e.g. `<https://youtube.com/...><!-- rf: card=embed width=100 -->`

Upload:

  * Relative links to markdown files (`[setup](./setup.md)`) are rewritten to `https://<site>/wiki/spaces/<KEY>/pages/<id>`, using the target file's frontmatter `id` and `space_key`.
  * `upload --config` runs in two passes: first create/update every page so each one has an ID, then write the bodies with links resolved. This lets new pages link to each other.
  * A link to a markdown file that isn't in the upload set and has no page ID fails the upload with an error listing the unresolved links. A broken `setup.md` link on a Confluence page is worse than a failed upload.
  * Absolute Confluence URLs pass through unchanged.

Fetch:

  * Stdout output keeps absolute URLs. LLMs can follow them directly, since `rfluence fetch` accepts URLs.
  * `rfluence fetch -o <path>` scans the project's markdown files for frontmatter page IDs and rewrites matching page URLs to relative paths. The project root is the directory containing `.rfluence.yaml`, or the output directory if there is none. The scan only happens for file output, so the stdout path stays fast.
  * Recognised same-site URL forms: `/wiki/spaces/KEY/pages/ID` (with or without the trailing title) and tiny links (`/wiki/x/...`). Links are matched by page ID only; the title part is ignored (it changes when a page is renamed, and Confluence adds or removes it on save).

What Confluence does with links (verified on test page 295349):

  * Text links and smart links (`inlineCard`) created via the API to page URLs render and work as page links.
  * Confluence rewrites page URLs on save, inconsistently:
    * Without an anchor, a trailing title is removed: `/pages/131074/rfluence+image+API+test` is stored as `/pages/131074`.
    * With an anchor, the title is added: `/pages/131074#A:-...` is stored as `/pages/131074/rfluence+image+API+test#A:-...`.
  * So the stored URL form can't be predicted. `upload` writes `https://<site>/wiki/spaces/<KEY>/pages/<id>[#anchor]`, `fetch` resolves any form to the page ID, and round-trip comparison uses the resolved target, not the URL text.
  * Same-page anchor `href`s (`#...`) are stored exactly as sent.
  * Moving a page to another space rewrites the space key in every page URL in its stored body to the target page's current space, without a new version (verified by moving the reference pages to space `rfluencete`; their children moved with them). So the space key in a stored URL can change at any time, another reason links are matched by page ID only.

Heading anchors:

  * The converter translates between GitHub-style (`#on-this-page`) and Confluence-style (`#On-this-page`) by finding the target heading and regenerating the anchor in the other style.
  * Confluence anchor algorithm (verified in the browser on page 295349):
    * Trim leading/trailing whitespace, then replace **each** space with `-` (not collapsed: `Extra   spaces` -> `Extra---spaces`).
    * Keep case, punctuation and non-ASCII characters (`Install-&-Setup-(v2.0)`, `FAQ:-What's-new?`, `Ünïcode-Héading`).
    * Duplicate headings get `.1`, `.2`, ... (second `Duplicate` -> `Duplicate.1`).
    * The browser's "copy link" percent-encodes the anchor like JavaScript's `encodeURIComponent` (`&` -> `%26`, `:` -> `%3A`, `?` -> `%3F`; `'`, `(`, `)` left as-is).
  * GitHub-style anchors (what LLM-written markdown uses): lowercase, remove punctuation except `-` and `_`, spaces -> `-`, duplicates get `-1`, `-2`, ....
  * Don't use the API's `body-format=view` HTML for anchors: it uses an older server renderer with different IDs (`<PageTitle>-<HeadingWithoutSpaces>`, e.g. `rfluencelinkAPItest-Onthispage`) that don't match what the browser uses.
  * `upload` writes anchors **raw** (`#Install-&-Setup-(v2.0)`, not `#Install-%26-Setup-(v2.0)`). Verified on page 295349: raw and percent-encoded anchors both work, for same-page and cross-page links. But only raw links show the browser's "visited" colour after clicking, because Confluence records the raw form in browser history. Raw is also more readable and needs no encoding step.
  * `fetch` percent-decodes anchors before matching them to headings, so links written either way (e.g. pasted from "copy link") are recognised.
  * Same-page links use the current document's headings. Cross-page links (`./setup.md#install`) need the target's headings: from the local file on upload, and from the project scan on fetch.

Smart links:

  * Confluence stores only a smart link's URL and shows the target's current title, so fetch looks the titles up: after the page, one request (`GET /wiki/api/v2/pages?id=<id>,<id>,...`, up to 250 IDs) for the pages on the same site that the page's smart links (inline, block and embed cards) point at. It's only made when there are such links (measured: 0.9 s instead of 0.6 s for the reference page), and it's best effort: if it fails, the links keep their URLs.
  * Written as `[Title](url)<!-- rf: card=inline -->` (or `card=block` / `card=embed` for cards on their own line). The comment marks the link as a smart link, so upload makes it an `inlineCard` again rather than a text link. Links whose title isn't known (other sites, non-page URLs, deleted pages) stay `<url>`, which is also a smart link. Simplified output writes `[Title](url)` without the comment.
  * The text is display-only: upload ignores it (Confluence keeps no text for a smart link), and so do round-trip comparisons and `fetch -o`'s check for local edits, since it changes when the target page is renamed or when the lookup fails. `<url>` and `[Any text](url)<!-- rf: card=inline -->` are the same content.
  * With `fetch -o`, a smart link to a page held by a project file gets that file's relative path (`[Setup](./setup.md)<!-- rf: card=inline -->`); the comment keeps it a smart link on upload.

Out of scope for now: links to non-markdown local files
## Decisions

### Confluence Cloud only, ADF as the page body format

Only Confluence Cloud is supported (no Data Center / Server). Page bodies are read and written as ADF (Atlassian Document Format, `body-format=atlas_doc_format` on the v2 pages API), not storage format (XHTML).

Why:

  * Cloud stores pages from the current editor natively as ADF. Using storage format would add a server-side ADF <-> storage conversion on every read and write, which is another source of silent changes that breaks round trips.
  * ADF is JSON, so it deserializes directly into Rust types with `serde`. Storage format is XML-like but not well-formed standalone (undeclared `ac:`/`ri:` namespaces, HTML entities), which makes parsing tedious.
  * ADF nodes (`heading`, `bulletList`, `codeBlock`, `taskList`, `panel`, ...) map closely to a Markdown AST, which keeps the converter simple.
  * The main thing ADF costs us is Data Center support, which is out of scope.

See "Mermaid diagrams (merfluence)" for how the Mermaid app's diagrams appear in ADF.

### Page links are tracked in frontmatter, with a title fallback

After `rfluence upload` creates or updates a page, it writes the page's details (`id`, `version`, `url`, ...) back into the file's `rfluence:` frontmatter (see "Frontmatter"). No lock file is used.

Why:

  * `version` has to be updated after every upload for the conflict check to work, so the file is modified regardless. Keeping everything in frontmatter means one place that is always current.
  * `rfluence fetch` already produces frontmatter, so fetched and uploaded files have the same shape, and single-file and `--config` uploads work the same way.
  * Each file carries its own link to its page; nothing breaks if files are moved or copied without a side file.

Fallback when the link is lost (e.g. an LLM rewrites a file and drops the frontmatter):

  * Before creating a page, `rfluence upload` looks for an existing page with the same title in the target space. Confluence requires titles to be unique within a space, so this can't silently create a duplicate.
  * If a page is found, `rfluence` refuses to upload and reports the matching page ID, because without a `version` there is no way to check for remote edits. `--force` overwrites that page and writes the frontmatter back.

### `upload --config` mirrors the directory structure as the page tree

Each `.rfluence.yaml` entry's files are placed in Confluence following their directory structure under the entry's `ancestor`, rather than flat under the ancestor or via an explicit `parent` in each file. See "Page hierarchy" for the rules.

Why:

  * Doc folders (`docs/api/auth.md`, `docs/cli/auth.md`) keep their structure in Confluence instead of ending up side by side.
  * It needs nothing from the author. Explicit per-file parents are tedious and LLMs won't maintain them.
  * Directory nodes are found by title, consistent with the title fallback above, so no lock file is needed.
  * Moves are opt-in (`--move`) so `rfluence` doesn't silently undo reorganizations done in Confluence.

### Remote pages are only deleted with `--prune`, and only if `rfluence` created them

Deleting a local file never deletes its Confluence page by default. `upload --config` reports orphans (remote pages with no local file), and `--prune` moves them to the trash. Only pages and folders `rfluence` created, marked with an invisible `rfluence` content property, can be pruned. See "Renames and deletions".

Why:

  * Pages people added under an uploaded tree in Confluence must never be touched. A label could be removed by accident in the UI; a content property is invisible and can hold structured data.
  * The property records the version `rfluence` last uploaded, so `--prune` can skip pages edited in Confluence since then. Deleting a local file can't silently trash someone's changes.
  * Deleted pages go to the trash and can be restored.

## Design constraints

### CLI performance

The CLI should optimize for fetch and search. This should be fast and snappy. The time it takes to run from the CLI, return a result, and exit should be considered. Since this could be an LLM tool call, we want it to be as quick as reasonable.

Rust startup is negligible; almost all latency is network round-trips. So:

  * Aim for one API call per command. Cache rarely-changing lookups (e.g. space key -> space ID) locally instead of resolving them on every run.
  * Use a current-thread tokio runtime with `reqwest` (or blocking `ureq`), with rustls to avoid loading OpenSSL.
  * Keyring lookups on Linux go over D-Bus and can prompt or hang (SSH, CI, agent sandboxes). Put a timeout on keyring access and never prompt in non-interactive commands.

### Libraries

Use widely adopted Rust libraries where reasonable. Candidates:

  * `clap` for argument parsing
  * `reqwest` (rustls) or `ureq` for HTTP
  * `serde` / `serde_json` for ADF and API types
  * `comrak` for markdown parsing and normalized CommonMark output
  * `serde_yaml_ng` or `serde_norway` for YAML (not `serde_yaml`, which has been unmaintained since 2024)
  * `globset` / `ignore` for upload globs
  * `keyring-core` for token storage, with one store crate per platform: `zbus-secret-service-keyring-store` on Linux (pure Rust D-Bus, so no libdbus at build time), `apple-native-keyring-store` on macOS, `windows-native-keyring-store` on Windows. (keyring 4 split the old `keyring` crate this way; without a registered store there is no keyring, and `rfluence` falls back to the token file.)
  * `ureq` for HTTP: blocking, rustls, one connection pool shared across threads. `rfluence fetch` requests the page and its attachment list in parallel on two threads, so it costs one round trip. Measured on the test site: `rfluence fetch` takes 0.44–0.64 s end to end, the same as a bare `curl` of the page request alone (0.49–0.67 s); nearly all of it is Confluence's response time.
  * `emojis` for emoji data (see "Emoji").
  * `insta` for snapshot tests, `wiremock` or `rvcr` for recorded HTTP responses

## Workspace layout

```
crates/
  rfluence-convert   # pure md <-> ADF conversion; no I/O; most tests live here
  rfluence-client    # Confluence API client, auth, config
  rfluence-cli       # binary: rf
```

The converter is the heart of the project. Keep it pure (no network or filesystem access) and test it heavily with snapshot and round-trip property tests.

## Useful info

The .env file has `CONFLUENCE_API_KEY`, `CONFLUENCE_EMAIL` and `CONFLUENCE_BASE_URL` env variables for testing.

The reference pages on the test site, captured as test fixtures, are described in `fixtures/confluence/README.md`: what each covers, how to re-capture them, and how to recreate them in another space or site if the test account goes away. Page IDs in this document (e.g. "verified on test page 98404") refer to that README's table.

## rfluence (the command)

The command for retrieving Confluence pages. The intended target is to use it in AI skills when searching for documents.

### Auth

rfluence keeps one account per Confluence site, like `gh auth`. A site is identified by its host; it can be given as a name (`example`, meaning `https://example.atlassian.net`), a host, or a URL.

Env-based auth requires all three variables:

  * `CONFLUENCE_API_KEY` the API token
  * `CONFLUENCE_EMAIL` the Atlassian account email (Cloud API tokens authenticate with email + token over HTTP Basic, so the token alone is not enough)
  * `CONFLUENCE_BASE_URL` the site URL (e.g. `https://example.atlassian.net`)

If only some are set, exit with an error naming the missing variables rather than silently mixing env and stored credentials.

Which site a command uses, first match wins:

  1. the site of a page URL given to the command (`rfluence fetch https://other.atlassian.net/wiki/...`);
  2. `--site <site>`;
  3. `RFLUENCE_SITE`;
  4. the `CONFLUENCE_*` variables' site, if they are set;
  5. the default site (the last one logged in to, or chosen with `rfluence auth switch`).

The `CONFLUENCE_*` variables are used for their own site only; any other site uses its saved account. A site with no account fails with "not logged in to <host>: run `rfluence auth login --site <host>`".

Storage: sites, emails and the default site in `~/.config/rfluence/auth.json`; each token in the system keyring (service `rfluence`, user `<email> on <base URL>`), or, when no keyring is available, in `~/.config/rfluence/tokens/<host>` (mode 0600). `RFLUENCE_CONFIG_DIR` uses another directory, and `RFLUENCE_NO_KEYRING=1` skips the keyring (CI, sandboxes, tests). The earlier single-account `auth.json` (`{base_url, email}` with the token in `token`) is read and migrated on the next login or logout.

### Commands

  * `rfluence search <query>` used to search Confluence. Returns a list of matches with a description for the LLM to evaluate.
    * Free text maps to CQL `text ~ "<query>" and type = page`. Flags: `--space` (repeatable: any of them), `--label` (repeatable: all of them), `--limit` (default 10), `--site`, and `--cql` for a raw CQL query instead of a query and filters.
    * Uses the v1 `/wiki/rest/api/search` endpoint (v2 has no search), one request. Highlight markers (`@@@hl@@@`) are stripped from excerpts, HTML entities decoded, and each excerpt put on one line, cut to about 220 characters. Measured: about 0.8 s.
    * Each result includes page ID, title, space, URL, last modified, labels, and excerpt (labels via `expand=content.metadata.labels`, no extra call). Text output, compact for LLMs:

      ```
      458790  rfluence ADF reference
        rfluencete · updated 2026-10-04 · labels: two, words
        https://example.atlassian.net/wiki/spaces/rfluencete/pages/458790
        API-created reference page for rfluence. Each section exercises...

      3 results. Read one with `rfluence fetch <id>`.
      ```

      `--json` gives `{total, results: [...]}`. No matches prints "No results." and exits 0; invalid CQL exits 2 with Confluence's reason.
  * `rfluence fetch <identifier>` used to fetch a specific page. Returns the page in markdown form. This could be used by an LLM, or by a user to download, modify, and re-upload. Includes frontmatter for where the page came from so that the upload command can easily upload this file back to Confluence.
    * Accepts a page ID, a full page URL, or `SPACE:Title` (titles are only unique within a space).
    * `--max-chars` and `--section <heading>` to avoid flooding an LLM's context with large pages.
    * Output is always the round-trip form by default, whether to stdout or a file (choosing by destination would confuse people). `--simplified` gives the reduced form for reading (see "Simplified output"); the rfluence skill tells LLMs to use it when they only need to read a page.
    * `-o <path>` writes to a file and downloads images to `<name>.assets/` next to it (see "Images and attachments"):
      * Only images (attachments shown by `mediaSingle` nodes) are downloaded, in parallel; a file already there with the same size is skipped, so fetching again downloads only what changed. Other attachments stay on Confluence. Measured on the test site: 1.9 s for a page with 3 new images (each download is two round trips: Confluence, then the media service it redirects to), 0.6 s when they're up to date.
      * Links (text links, image links, and titled smart links) to pages held by markdown files in the project become relative paths, with anchors translated to the target file's GitHub-style anchors (see "Links"). Smart links without a looked-up title keep their URLs, since `<./setup.md>` isn't a link. The project scan skips files ignored by `.gitignore`.
      * An existing file keeps its non-`rfluence` frontmatter keys, byte for byte, and its `weight`.
      * Local changes aren't overwritten: if the file is for the same page, its body is compared with what the version in its frontmatter converts to (fetching that version only if Confluence has a newer one). The comparison is in normalized form and ignores link destinations and image folders, which change when project files are added or the file is renamed; so a change to only a link's target isn't detected. With local changes, or a file holding another page, `fetch -o` refuses (exit 6) unless `--force`.
      * Can't be combined with `--section` or `--max-chars` (it would replace the file with part of the page). `--simplified -o` writes the simplified form and downloads nothing.
  * `rfluence auth`, modelled on `gh auth` (see "Auth"):
    * `rfluence auth login [--site S] [--email E] [--with-token]` logs in to a site and makes it the default. It prompts for anything not given (the token without echo); `--with-token` reads the token from standard input for scripts. The credentials are checked against Confluence before anything is saved.
    * `rfluence auth logout [--site S]` removes a site's account and token (default: the default site); another saved site becomes the default.
    * `rfluence auth status [--site S]` lists each account (and the `CONFLUENCE_*` site, if set), marks the default, shows where its token is, and checks it against Confluence. Exits 4 if any check fails.
    * `rfluence auth token [--site S]` prints the token rfluence would use.
    * `rfluence auth switch [--site S]` changes the default site; without `--site` it switches between two saved sites.
  * `rfluence upload <path>` uploads an MD file to Confluence using the frontmatter data.
    * If the remote page version is newer than the `version` in the frontmatter, refuse to upload unless `--force` is passed. This prevents overwriting edits made in Confluence.
    * A file without a page ID creates a new page; see "Frontmatter" > "New pages" (`--space` / `--parent` fill in missing values). If a page with that title already exists in the space, refuse unless `--force` is passed (see Decisions).
    * Frontmatter is stripped before upload. After a successful upload, the `rfluence:` block is written back (see "Frontmatter" > "Reading and writing").
    * `--dry-run` shows what would be created/updated without changing anything.
    * Runs the same checks as `rfluence check`: warnings are printed and the upload goes ahead; errors stop the upload before anything is sent, unless `--force` (which uploads the approximations listed in "Checking markdown").
    * Everything that can fail locally fails before anything is sent: `rfluence:` keys (unknown keys, `simplified` or `partial` files), labels, check errors, links to files without a page, and image files that are missing with no attachment of that name. The body is converted once with stand-in IDs for images not uploaded yet, then again with the real `fileId`s.
    * Requests: the page (body, labels), its attachments and its `rfluence` property, in parallel; then new attachments and attachment versions; the body (`PUT /wiki/api/v2/pages/{id}` with version + 1); new labels; the property, if the page has one.
    * Nothing is sent for the body if it hasn't changed. The page and the upload are both converted to markdown with the same settings (so what Confluence adds on save doesn't count) and compared, with page links compared by page ID and anchor (Confluence adds or removes the title in stored page URLs). Verified: a PUT whose body only differs in a link URL's title part doesn't create a version either; the response has the old version number, and upload reports the page as up to date.
    * Images: a local file is compared with the page's attachment of the same name by size, then (same size) by downloading it. Changed images get a new attachment version, new ones are attached, unchanged ones and ones whose file is missing reuse the attachment.
    * Output: what was (or with `--dry-run`, would be) changed: the version, title, images uploaded and updated, labels added, and inline comments that are detached. `--json` gives the same as an object. Exit 6 when the page changed in Confluence since the file was fetched.
    * Measured on the test site: 0.6 s for `--dry-run` or an unchanged page without images, 1.4 s for an unchanged page with an image (the download to compare it), 2.4–3 s when a body, an image and labels are sent.
    * Moving a page (a `parent` that differs from the page's) isn't done by single-file upload yet: it warns.
  * `rfluence upload --config <path>` uploads multiple pages using a config file. The config file (`.rfluence.yaml` in the project root) is a YAML list of entries, each mapping files (exact paths or globs) to a Confluence space and ancestor page, with optional labels. See "Upload config".
  * `rfluence diff <path>` shows the differences between a local file and the current remote page.
  * `rfluence check <path>...` reports, without network access, what upload would approximate (warnings) or can't represent (errors); see "Checking markdown". Output is `path:line: severity: message` lines and a summary (`--json` for structured output). Exits 1 if there are errors. It also warns about local images whose file is missing.

### Simplified output

`rfluence fetch --simplified` writes markdown for reading, not for round trips: fewer characters, nothing an LLM has to skip past. Compared with the round-trip form:

| Round-trip form | Simplified |
| --- | --- |
| `rf:` settings comments (widths, alignment, cards, ...) | dropped |
| Layout column markers | dropped; columns read in order |
| `<span data-adf='...'>IN PROGRESS</span>` (status, mention, date, ...) | the visible text: `[IN PROGRESS]`, `@Chris Edwards`, `2027-01-01` |
| ```` ```adf ```` blocks | the text inside them (a decision list's decisions, a custom panel's content), or a one-line marker for macros without text (`[Table of contents]`, `[Child pages]`) |
| Colours, underline, sub/superscript as inline HTML | plain text |
| `![alt](page.assets/x.png)` with settings | `[image: alt]` (or the file name), since the file isn't there; external images keep their URL |
| `<details><summary>Title</summary>` ... `</details>` | **Title** as a bold line, then the content |
| `[Title](url)<!-- rf: card=inline -->` (smart links) | `[Title](url)` |
| `rfluence:` frontmatter for round trips | `title`, `url`, `space`, `labels`, last updated, and `simplified: true` |

Kept as they are: headings, lists, tables (GFM or HTML), code blocks, Mermaid source, links, emoji.

`rfluence fetch --section` and `--max-chars` give part of a page (`--max-chars` cuts at a block boundary and ends with a note listing the page's sections, so an LLM can ask for one). Their output has `partial: true` in its frontmatter and can't be uploaded either.

On stdout there is no file name to put images next to, so `rfluence fetch` writes image paths into a folder named after the page title (`<title-slug>.assets/`). Uploading such a file without the images reuses the attachments with those names (see "Images and attachments").

Simplified output can't be uploaded: it has dropped content the page still has, so uploading an edited copy would delete it. `rfluence upload` refuses a file whose frontmatter has `simplified: true`, with a message to fetch it again without `--simplified`.

### Frontmatter

rfluence's keys live under an `rfluence:` key so they don't collide with frontmatter the LLM or user already wrote. This section is the single reference for the schema; other sections link here.

```yaml
---
rfluence:
  id: "123456"
  space_key: ENG
  parent: "123000"
  title: Page title        # only when needed, see "Title"
  version: 7
  url: https://example.atlassian.net/wiki/spaces/ENG/pages/123456
  labels: [architecture, ai-generated]
  weight: 10               # optional, user-set
---
```

#### Keys

| Key | Type | Written by | Used by upload | Description |
| --- | --- | --- | --- | --- |
| `id` | string | `rfluence` | yes | Page ID. Absent means "create a new page" (subject to the title fallback in Decisions). |
| `space_key` | string | `rfluence`; user for new pages | yes | Space key. For files in `.rfluence.yaml`, must match the entry's `space_key`; a mismatch is an error (moving pages between spaces isn't supported). |
| `parent` | string | `rfluence`; user for new pages | single-file only | Parent page or folder ID. For files in `.rfluence.yaml`, the mirrored tree decides the parent and this is just a record. For single-file `rfluence upload`, a `parent` that differs from the page's current parent follows the `--move` rule (warn; `--move` moves it). |
| `title` | string | user; `rfluence` when needed | yes | Page title override. See "Title". |
| `version` | integer | `rfluence` | yes | Page version after the last fetch/upload. Upload refuses if the remote version is newer, unless `--force`. |
| `url` | string | `rfluence` | no | Page URL, for people and LLMs to follow. Ignored on upload. |
| `labels` | list of strings | `rfluence` and user | yes | Labels. Upload is additive (see "Labels"). |
| `weight` | integer | user only | `--config` only | Sibling order (see "Child page order"). Confluence has no equivalent, so `fetch` never writes it. |

  * IDs are strings (quoted in YAML) so they aren't parsed as numbers.
  * Unknown keys under `rfluence:` are an error, so typos are caught (same as `.rfluence.yaml`).
  * Content IDs are shared between pages and folders, so `parent` doesn't need a type.

#### Title

Confluence titles are separate from the page body, but LLM-written markdown usually starts with `# Title`. The rules keep both directions stable:

  * Upload: if `title` is not set and the body starts with an H1, that H1 is the page title and is removed from the body (so Confluence doesn't show the title twice). If `title` is set, it is the page title and the body is uploaded as-is, including any leading H1.
  * Fetch: if the page body doesn't start with an H1, write `# <page title>` at the top and no `title` key. If the body does start with an H1 (e.g. a different heading added in Confluence), write the `title` key so the next upload doesn't mistake that H1 for the title.
  * A file with neither `title` nor a leading H1 is an error for new pages. For an existing page, upload keeps its current title.

#### Reading and writing

  * `rfluence fetch` (stdout and `-o`) writes `id`, `space_key`, `parent`, `version`, `url`, `labels`, and `title` when the Title rules require it.
  * After a successful upload, the `rfluence:` block is rewritten to exactly what `rfluence fetch` would produce for the page (including the page's full label set and the new version), plus the user-only `weight`. This keeps `fetch(upload(md))` and the written-back file identical.
  * `rfluence` only rewrites the `rfluence:` block. Other frontmatter keys, their order and comments are left byte-for-byte unchanged. Inside the block, keys are written in the order shown above, and comments in it are not preserved.
  * `rfluence fetch -o` over an existing file keeps that file's non-`rfluence` frontmatter keys and its `weight`, since Confluence doesn't store them.
  * Upload strips all frontmatter from the body. Non-`rfluence` keys are never sent to Confluence, so round-trip comparison covers the body and the `rfluence:` block only.

#### New pages

A file without `id` creates a page. It needs:

  * a space: `space_key`, the `.rfluence.yaml` entry, or `rfluence upload <path> --space <KEY>`;
  * a parent: `parent`, the mirrored tree, or `--parent <id>` (defaults to the space homepage if none is given);
  * a title: see "Title".

Flags only fill in missing values; they don't override frontmatter.

How a page is created (`rfluence upload`, implemented):

  * Before anything is sent, the same local checks as for updates, plus: a title, a space, and every image file present (there are no attachments to reuse).
  * The space (`GET /wiki/api/v2/spaces?keys=<KEY>`, for its ID and homepage) and a page with the same title in it (v1 `content?spaceKey=&title=`) are looked up in parallel. If the title is taken, upload refuses (exit 6) and names the page, since the file may have lost its `id` (see "Page links are tracked in frontmatter, with a title fallback"); with `--force` it updates that page instead, as if the file had its `id` (the version check is skipped, and the page doesn't get the `rfluence` property: `rfluence` didn't create it).
  * Without images: one `POST /wiki/api/v2/pages` with the body (version 1).
  * With images: create the page with an empty body, write its `id` and version to the file straight away (so if a later step fails, the next upload updates that page instead of refusing over its title), attach the images, then write the body (version 2). Verified: an empty `doc` is accepted as a page body.
  * Then labels and the `rfluence` property (in parallel), and the `rfluence:` block is written back as for updates.
  * Mermaid diagrams: as for updates, the form depends on the apps installed on the site (see "Mermaid diagrams on sites without merfluence"); the lookup runs in parallel with the space and title lookups.
  * Measured on the test site: about 3 s to create a page with an image and a label (lookups, create, attach, body, then labels and property).

#### Relation to the content property

The `rfluence` content property on the page (see "Renames and deletions") records the version `rfluence` last uploaded. Right after an upload it equals the frontmatter `version`. If the remote page version is later higher than both, the page was edited in Confluence.

### Upload config

The config file is `.rfluence.yaml` in the project root, modelled on md2c's `.md2c.yaml`. The file is a **YAML list**; each entry maps a set of markdown files to a place in Confluence.

| Field | Required | Description |
| --- | --- | --- |
| `paths` | one of `paths` / `globs` | Exact file paths, relative to the project root. A listed file that doesn't exist is an error. |
| `globs` | one of `paths` / `globs` | Glob patterns, e.g. `docs/**/*.md`. |
| `exclude` | no | Glob patterns removed from the matches (rfluence addition; md2c has none). |
| `space_key` | yes | Confluence space key, e.g. `ENG`. |
| `ancestor` | one of `ancestor` / `ancestor_id` | Title of the parent page (or folder) in the space that the entry's files go under. |
| `ancestor_id` | one of `ancestor` / `ancestor_id` | Page or folder ID of the parent, for when the title is ambiguous or may be renamed in Confluence (rfluence addition). |
| `labels` | no | Labels added to every page in the entry (see "Labels"). |
| `root` | no | Directory that maps to the ancestor (rfluence addition). Defaults to a root computed from the config text; see "Entry root". |
| `folder_title` | no | Template for the titles of folders created for directories (rfluence addition). Default `{dir}`. See "Folder titles". |

```yaml
# .rfluence.yaml
- globs:
    - how-to/github/**/*.md
  exclude:
    - how-to/github/drafts/**
  space_key: ENG
  ancestor_id: "123000"   # "Engineering Docs"; an ID keeps working if the page is renamed
  labels:
    - github
    - ai-generated
```

With these files:

```
how-to/github/
  README.md            # "# GitHub How-tos"
  01-setup.md          # "# Setting up GitHub"
  02-workflow.md       # "# Our GitHub workflow"
  actions/
    runners.md         # "# Self-hosted runners"
    secrets.md         # "# Managing secrets"
  drafts/
    wip.md             # excluded
```

the upload produces this page tree:

```
Engineering Docs (ancestor)
  GitHub How-tos              page, from README.md (the root's index page)
    Setting up GitHub         page
    Our GitHub workflow       page
    actions                   folder (no index.md in actions/)
      Self-hosted runners     page
      Managing secrets        page
```

A single entry is enough: the README becomes the parent page and everything nests under it. Unlike md2c, there's no need for a separate entry that uploads the README first and a second entry that uses its title as `ancestor`.

Rules:

  * Each entry needs at least one of `paths` / `globs` (both are allowed), and exactly one of `ancestor` / `ancestor_id`.
  * Unknown keys are an error, so typos are caught.
  * Paths and globs are relative to the directory containing the config file (the project root), so `--config other/dir/.rfluence.yaml` works the same way.
  * Only `.md` files are matched; other files matched by a glob are ignored. `**` matches across directories (including zero directories). Files ignored by `.gitignore` are skipped (`ignore` crate).
  * `exclude` applies to `globs` and `paths`. A file listed in `paths` that also matches `exclude` is an error, since the entry contradicts itself.
  * A file matched by more than one entry is an error (its destination would be ambiguous). The error suggests using `exclude` in one of the entries.
  * `ancestor` is resolved by title in the space. Because a page and a folder can share a title, if both exist the upload fails and asks for `ancestor_id`. Titles are fragile: if the page is renamed in Confluence the upload fails, and if another page later takes that title, files would go to the wrong place. Normal output and `--dry-run` always show the resolved ancestor (title and ID), and `ancestor_id` is recommended for long-lived configs.
  * The ancestor can be a page uploaded by another entry in the same file, in any order. Unlike md2c, entries don't need to be ordered parent-first and nothing needs to be run twice: the two-pass upload (see "Links") creates all pages before resolving ancestors and links.
  * An ancestor that neither exists in Confluence nor is created by the upload is an error.
  * Page titles come from frontmatter `title`, falling back to the leading H1 (md2c uses the H1); see "Frontmatter" > "Title".
  * Unlike md2c, files are not placed flat under the ancestor: the directory structure is mirrored (see "Page hierarchy").

#### Entry root

The entry root is the directory that maps to the ancestor. It is computed from the config text, not from which files currently exist, so the tree only changes when the config changes. (Computing it from the matched files would shift the root, and move every page, whenever a file is added or removed at the top level.)

  * `root`, if set. Every matched file must be inside it; otherwise it's an error.
  * Otherwise, the deepest common directory of:
    * each glob's static prefix: the directories before the first component containing a wildcard (`*`, `?`, `[`, `{`). `how-to/github/**/*.md` -> `how-to/github/`;
    * each listed path's directory.

#### Folder titles

Directories without an `index.md` / `README.md` become folders, and folder titles must be unique across the whole space. Common directory names (`api`, `images`, `guides`, `reference`) will collide as soon as two projects or entries share a space.

  * `folder_title` is a template for folder titles. Variables: `{dir}` (directory name), `{parent}` (title of the parent page or folder in the tree), `{path}` (directory path relative to the entry root, e.g. `api/v2`). Default: `{dir}`.
  * Example: `folder_title: "{parent} / {dir}"` turns `actions/` above into the folder `GitHub How-tos / actions`.
  * On a folder title collision, the pre-flight error suggests setting `folder_title` or adding an `index.md` to the directory.

#### How `upload --config` runs (implemented)

`rfluence upload --config [FILE]` (default: `.rfluence.yaml` in the current directory or the nearest one above it; `--dry-run`, `--force` and `--site` work as for single files):

  1. Load and check the config, and match the files (all errors at once).
  2. Look up each entry's space and, for `ancestor_id`, the ancestor (page or folder; it must be in the entry's space). Build the tree (titles, folders, order).
  3. Check every file as single-file upload would (frontmatter, labels, `rfluence check` errors, links, missing images of new pages), and report every problem before anything is sent. Links to files of the upload that have no page yet are allowed: they get one in pass 1.
  4. Resolve `ancestor` titles: a page or a folder in the space (both: an error asking for `ancestor_id`), else a page of this upload, else an error.
  5. Files without an ID whose title is already a page in the space: refused (exit 6), all listed, unless `--force`, which uploads to those pages.
  6. Pass 1, top-down: a folder is found among its parent's children by title (`direct-children`, filtered by type), else created; a folder with that title elsewhere in the space is an error (folder titles are unique per space). New pages are created empty under their tree parent and their `rfluence:` block written at once, with the `rfluence` property (`config_labels` from the entry). Created folders get the property too.
  7. Pass 2: every file is uploaded as single-file upload does (version check, images, links, inline comments, labels), plus the entry's labels; the property's `config_labels` is kept current. A page that isn't under its tree parent gets a warning and is left where it is (see "Page hierarchy"; `--move` comes later).
  8. Output: each entry's ancestor and tree (`[new page]`, `[page 123]`, `[new folder]`, ...), a line per file (`created`, `updated (version 3 -> 4)`, `up to date`, labels added, ...), and totals. `--dry-run` shows the same without changing anything.

Not implemented yet: placing new pages in order and reordering (see "Child page order"), `--move`, `--prune` and `--prune-labels`.

API findings (test site): CQL `type = folder and space = "KEY" and title = "..."` finds folders by title; the v1 content API can't (`GET /wiki/rest/api/content?type=folder` returns 501 "Cannot fetch folders with ContentFinder"). `GET /wiki/api/v2/{pages|folders}/{id}/direct-children` lists pages and folders together with `type`, `title` and `childPosition`. Measured: a tree of 3 new pages and a folder takes about 6 s (each page is created, then its body uploaded); the same tree up to date, under 1 s.

#### Project-wide settings

The file stays a top-level list (like md2c), so there's no place for project-wide settings. This is deliberate for now:

  * Settings go per entry (e.g. the future `repo_url`) or in user config (`~/.config/rfluence/`). The site URL comes from auth, and the merfluence app IDs are discovered automatically.
  * If project-wide settings become necessary, a second form will be accepted alongside the list: a mapping with `settings:` and `entries:` (the list). Existing list-form files stay valid, so nothing breaks.

### Page hierarchy

`upload --config` mirrors the local directory structure as the Confluence page tree under each entry's `ancestor` (see Decisions).

  * The entry root (see "Upload config" > "Entry root") maps to the ancestor. Files directly in the root become children of the ancestor, and subdirectories become pages or folders below it. If the root itself has an `index.md` / `README.md` in the entry, that page goes under the ancestor and everything else nests under it.
  * A directory containing `index.md` or `README.md`: that file **is** the directory's page (its content becomes the page) and the directory's other files and subdirectories become its children.
  * A directory without one: becomes a Confluence **folder** (v2 `/wiki/api/v2/folders`), titled with the entry's `folder_title` template (default: the directory name).
  * Directory nodes are found by title under their expected parent (`/wiki/api/v2/folders/{id}/direct-children` or `/wiki/api/v2/pages/{id}/direct-children`) and created if missing. No IDs are stored for directories, so no lock file is needed. Lookups must filter by type, because a page and a folder can have the same title.
  * Before uploading anything, every title in the upload set is computed (files and directories) and checked for collisions, separately for pages and for folders. On a collision, fail with a list of the clashes; the fix is a `title` in one file's frontmatter (or an `index.md` with a different title for a directory).
  * New pages get the parent from the tree. For existing pages whose current parent differs from the tree (reorganized in Confluence, or the file moved locally), `upload` warns and leaves the page where it is. `--move` moves pages to match the local tree; `--dry-run` lists the moves first.
  * Moving a page bumps its version, so after a move the new `version` is written back to the frontmatter along with `parent`.
  * Precedence: for files covered by a config, the config tree decides the parent. Single-file `rfluence upload <path>` uses `parent` from the frontmatter (see "Frontmatter").

Verified on the test site (folder 131110):

  * Folders can be created via the API, nested in other folders, and pages can be created inside folders (`parentId` = folder ID).
  * Page titles are unique among pages in a space, and folder titles are unique among folders in a space, regardless of parent. A page and a folder **can** share a title.
  * Collisions return HTTP 400 `BAD_REQUEST` with titles `A page with this title already exists: ...` or `A folder exists with the same title in this space`.
  * A page can be moved by `PUT /wiki/api/v2/pages/{id}` with a new `parentId` and the next version number, without sending the body. The body is kept and the version is bumped.

### Child page order

Confluence keeps an order for sibling pages and folders. `upload --config` makes the order of `rfluence`-managed siblings match the local order. This costs extra API calls, which is acceptable: performance matters for reads (`fetch`, `search`), not for `upload --config`.

Local order:

  * Siblings are sorted by file or directory name using natural sort, case-insensitive, so numeric prefixes work as expected (`2-setup.md` before `10-faq.md`).
  * An optional `weight` (integer) in a file's `rfluence:` frontmatter overrides this: lower weights come first, ties are broken by name, and files without a weight come after weighted ones.
  * A directory's position comes from its `index.md` / `README.md` (`weight`, else the directory name). Directories that became folders sort by directory name.
  * `index.md` / `README.md` aren't siblings of the other files; they are the directory's page.

Applying it:

  * New pages and folders are placed in position when they're created: Confluence appends new children at the end, so `rfluence` creates the page and then moves it after its predecessor (one extra call).
  * Existing pages that are out of order: `upload` warns and leaves them, and `--move` reorders them. This is the same rule as parent changes (see "Page hierarchy"), so `rfluence` doesn't silently undo reordering done in Confluence. `--dry-run` lists the moves.
  * Only `rfluence`-managed siblings (those with the `rfluence` content property) are ordered among themselves. Pages people added are left where they are, so they may end up interleaved.
  * Moves are minimized: read the parent's current child order, keep the longest run of managed siblings already in the right relative order, and move only the rest, each placed after its predecessor in the desired order.
  * API: read the order from `GET /wiki/api/v2/pages/{id}/direct-children` or `.../folders/{id}/direct-children`, which returns pages and folders together, sorted by `childPosition` (an opaque, sparse integer; use it only for comparing). Move with v1 `PUT /wiki/rest/api/content/{id}/move/{before|after}/{sibling id}` (no body; returns 200 `{"pageId": ...}`).

Verified on the test site (folder 262167):

  * The move API works for pages and for folders (despite the v1 `content` path), inside a folder, with pages or folders as the target: page before page, folder before folder, and page after folder all worked.
  * Reordering does **not** bump the version of pages or folders, so no write-back is needed and it can't cause version conflicts.
  * New children are appended at the end (positions increase in creation order).

### Labels

Labels live in frontmatter as `rfluence.labels` (see Frontmatter). Only global labels are handled; `my:` / `team:` prefixed labels are ignored.

  * Fetch: labels are written to the frontmatter, including for stdout output (useful context for an LLM). `GET /wiki/api/v2/pages/{id}?body-format=atlas_doc_format&include-labels=true` returns body and labels in one call; follow `labels.meta.hasMore` if a page has many labels.
  * Upload is **additive**: labels in the frontmatter that are missing in Confluence are added; labels are never removed by default. `--prune-labels` removes Confluence labels that aren't in the frontmatter (same opt-in pattern as `--move`).
  * Why additive: adding or removing labels does not bump the page version (verified), so the version check can't detect labels added in Confluence after a fetch. Exact sync would silently delete them.
  * `.rfluence.yaml` entries can list `labels` that are added to every page in the entry (e.g. `ai-generated`).
  * Config labels are recorded in the page's `rfluence` content property (`config_labels`). Because the full label set is written back to frontmatter after upload, removing a label from the config wouldn't otherwise remove it anywhere. With `--prune-labels`, a label that is in `config_labels` but no longer in the entry's `labels` is removed from Confluence and from the frontmatter, even though the frontmatter still lists it.
  * Before upload, labels are normalized: lowercase, and spaces -> `-`. Labels containing disallowed characters fail the upload with a clear error before anything is sent. The normalized labels are written back to the frontmatter so the next fetch matches.
  * API: add with `POST /wiki/rest/api/content/{id}/label` (`[{"prefix": "global", "name": "..."}]`), remove with `DELETE /wiki/rest/api/content/{id}/label?name=<url-encoded name>` (the query form works for names containing `/`). v2 can only read labels.

Verified on test page 524309 (since trashed; the resulting labels are kept on the `adf-reference` fixture):

  * Uppercase is lowercased (`UPPER` -> `upper`).
  * Spaces and commas silently **split** a label into several (`two words` -> `two` + `words`; `comma,label` -> `comma` + `label`). This is why `rfluence` must normalize spaces before sending.
  * Allowed: letters (including non-ASCII, e.g. `ünïcode`), digits, `-`, `_`, `/`.
  * Rejected with a generic HTTP 400 (`Could not add labels to content`, no reason given): `.`, `:`, `&`, `#`, `(`, `!`. Since the error doesn't say why, `rfluence` validates locally and reports the bad character itself.
  * Adding and removing labels leaves the page version unchanged.
  * New labels are searchable via CQL (`label = "dash-ok"`) immediately.

### Renames and deletions

Renaming or moving files:

  * Renaming a file has no effect on Confluence: the page ID is in the frontmatter and the title comes from frontmatter or the first H1, not the filename.
  * Moving a file to another directory changes its parent in the tree; handled by the `--move` rule in "Page hierarchy".
  * Changing a title (frontmatter `title` or H1) renames the page on upload, subject to the normal collision check. Links keep working because they use page IDs.
  * Links in other local files that point at a renamed file become unresolved, and `upload` fails with the list (see "Links").
  * Renaming a directory without an `index.md` renames its folder. The API can't rename folders (verified), and folders are found by title, so `rfluence` detects the rename through the directory's pages: their IDs are known, and if they all currently sit in an `rfluence`-managed folder with a different title, that is the old folder. With `--move`: create the new folder, move the pages into it, then trash the old folder once it's empty. Without `--move`: warn.

Deleting files:

  * Single-file `rfluence upload` never deletes anything.
  * `upload --config` lists the descendants of each entry's ancestor and reports orphans: `rfluence`-managed pages whose ID isn't in any local file's frontmatter, and `rfluence`-managed folders with no matching directory.
  * `--prune` moves orphans to the trash. `--dry-run` lists them first.
  * `--prune` skips (and reports) orphans that:
    * have been edited in Confluence since `rfluence` last uploaded them (page version != `version` in the `rfluence` property), unless `--force` is passed;
    * have child pages that aren't `rfluence`-managed, because deleting a page moves its children up a level (verified), which would silently reorganize pages people added.
  * Folders are only pruned once empty.

Content property (`rfluence`), written on every page and folder `rfluence` creates, and updated (`version`, `path`) whenever it uploads a new version of one. Pages `rfluence` didn't create never get it, even when it uploads to them (e.g. a fetched page sent back with `rfluence upload`), so they can never be pruned:

```json
{ "managed": true, "version": 7, "path": "docs/api/auth.md", "config_labels": ["github", "ai-generated"] }
```

  * `version`: the page version after `rfluence`'s last upload. `path`: the local path relative to the project root (for reporting). `config_labels`: labels added from the `.rfluence.yaml` entry (see "Labels").
  * API: `POST /wiki/api/v2/pages/{id}/properties` (or `/folders/{id}/properties`) with `{"key": "rfluence", "value": {...}}`; update with `PUT .../properties/{property id}` and the property's next version number (properties have their own version counter).
  * `GET /wiki/api/v2/pages/{id}?body-format=atlas_doc_format&include-labels=true&include-properties=true` returns body, labels and properties in one call. Confluence adds its own properties (e.g. `page-title-property-published`), so filter by key.

Verified on the test site (folder 262167):

  * Content properties can be set on pages and folders. Creating or updating a property does not bump the page version, so writing it after an upload causes no false version conflicts. Properties survive page moves.
  * Folders can't be renamed via the API: v2 `PUT /wiki/api/v2/folders/{id}` fails (`body is required`, then `No referenceId found` with a body), and v1 `PUT /wiki/rest/api/content/{id}` returns 501 `Cannot validate update of content of type folder`. The fallback (new folder, move pages, trash the old one) works.
  * `DELETE /wiki/api/v2/pages/{id}` and `DELETE /wiki/api/v2/folders/{id}` return 204 and move the item to the trash (`status: "trashed"`).
  * Deleting a page with child pages moves the children up to the deleted page's parent. Deleting a folder that still contains pages does the same: the pages move up, they aren't deleted.
  * Trashed titles don't block reuse: a new page or folder can be created with the title of a trashed one.

### Output and errors

  * Default output is compact text for LLMs; `--json` for structured output.
  * `upload --config --dry-run` prints the resolved plan as a tree: each entry's resolved ancestor (title and ID), then every file -> page title -> parent, with planned creates, updates, moves, reorders, label changes and prunes marked. This is the main way to check a config does what was intended.
  * Results go to stdout, errors go to stderr, with distinct exit codes: 0 success; 1 `rfluence check` found errors; 2 usage or configuration (bad arguments or page reference, missing or partial credentials); 3 not found (page, title or `--section`); 4 authentication failed; 5 other Confluence API or network errors; 6 the command would lose changes (`fetch -o` over local edits or a file holding another page; upload's version conflicts will use it too).

## Testing

For testing, generate some MD pages that are indicative of what LLMs produce. Include Mermaid diagrams and the other features listed under "Markdown features LLMs commonly produce". For e2e testing these will need to be uploaded to a test space, then downloaded. Support recorded responses for faster testing with a way to enable live testing.

  * Most fidelity testing runs offline against `rfluence-convert`, using real ADF captured from Confluence as fixtures.
  * Recorded HTTP responses via `wiremock` or `rvcr`; set `RFLUENCE_LIVE=1` to hit the real API. Strip auth headers from recordings.
  * Live e2e runs add a unique run ID to page titles (titles must be unique within a space) and clean up created pages afterwards. Be mindful of rate limits.

## Open questions

None currently.

## Suggested implementation order

  1. Build `rfluence-convert` and its test corpus.
  2. `rfluence fetch` and `rfluence search`.
  3. `rfluence upload` with version checks and creating pages (done), then `rfluence upload --config`.

## Future considerations

Not required now; ideas to revisit later.

### `rfluence pull`: fetch a page tree into a directory

The reverse of `upload --config`: fetch a page and all its descendants into a directory structure, following the same mirror rules (a page with children becomes a directory with an `index.md`, a folder becomes a directory). This would complete the round trip for whole projects, not just single pages. Could also write a matching `.rfluence.yaml` entry.

### Repository URL fallback for unresolved links

Currently a link to a markdown file that isn't in the upload set and has no page ID fails the upload (see "Links"). md2c instead converts such links into full URLs to the file in the source repository (e.g. GitHub).

  * Add an optional `repo_url` field to `.rfluence.yaml` entries (e.g. `https://github.com/org/repo/blob/main`). When set, unresolved links become `<repo_url>/<path relative to the project root>` instead of failing the upload.
  * Could be inferred from the git remote and current branch, but an explicit setting is more predictable (forks, non-default branches, CI checkouts).
  * Report each rewritten link in the upload output, so it's clear which links point outside Confluence.

### Non-image attachments and links to local files

Links to non-markdown local files (`[spec](./spec.pdf)`) and non-image attachments (PDFs, spreadsheets, ...) are currently out of scope. For now, fetch writes non-image attachments as plain links.

  * Upload: a relative link to a local non-markdown file would upload it as an attachment (same rules as images: named after the file, new version only if the content changed) and link to the attachment.
  * Fetch: attachment links and file cards (`mediaGroup`) would become relative links to `<name>.assets/<file>`, downloaded only with `-o`.
  * Decide whether file cards (`mediaGroup` / `mediaSingle` with non-image files) need their own markdown form, e.g. a link plus an `rf: card=file` comment.

### `rfluence fetch --comments`

Inline comments are left out of fetched markdown (see "Inline comments"), but their text can be useful context for an LLM reviewing a page. `rfluence fetch --comments` would add the comment threads as read-only output, e.g. as footnotes on the commented text, with author and date. Upload would ignore them; it re-anchors from the remote page as usual. Page (footer) comments could be included the same way. Costs one extra API call (`/wiki/api/v2/pages/{id}/inline-comments`), so it stays opt-in.

### Representing what `rfluence check` reports as errors

Each error in "Checking markdown" could get a mapping if it turns out to be common in LLM-written pages. Footnotes are the likeliest: e.g. superscript reference numbers plus a "Footnotes" section, written back as markdown footnotes on fetch.

### Mermaid apps with other IDs

Upload recognises merfluence and Mermaid Diagrams Viewer by their app IDs (see "Mermaid diagrams on sites without merfluence"). A fork, or another Mermaid app with the same macro form, has a different app ID. Users won't know app IDs, so this would be a setting made once per site (e.g. stored with the site's account in the config directory, or an environment variable such as `RFLUENCE_MERMAID_APP=<app id>`), with detection still the default.

### Jira content

The test site has no Jira, so Jira issue macros, Jira smart links and Jira issue lists haven't been observed. Until then they are handled like any other unsupported node (preserved as ```` ```adf ```` blocks; smart links as `inlineCard` / `blockCard`). With a Jira-connected test site: capture each form, decide whether any deserve a markdown form (e.g. `[PROJ-123](<issue url>)` for an inline issue link), and check what an LLM most needs from them on fetch (issue key, summary, status).

## support natural language search

Investigate and see if the api support getting search results using Confluence's Rovo. If it is, add a --nl option to the search.
