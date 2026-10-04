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

### Content Confluence has that markdown doesn't

Pages edited by humans in Confluence will contain things markdown can't express (Jira macros, status lozenges, @mentions, layouts, other app macros). `fetch` must not drop these, or the next `upload` will delete them. Unsupported nodes are preserved as opaque fenced blocks containing the raw ADF JSON (e.g. ```` ```adf ````), which `upload` passes back unchanged.

### Markdown features LLMs commonly produce

These should map to native Confluence elements where possible:

  * Mermaid code blocks -> merfluence macro
  * GitHub alerts (`> [!NOTE]`, `> [!WARNING]`, ...) -> Confluence info/note/warning panels
  * Task lists (`- [ ]`) -> Confluence task lists
  * Code fences with language tags -> code blocks with language
  * Tables (including pipes inside inline code), nested lists mixed with code blocks, footnotes, emoji, inline HTML

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
  * Upload: building the node requires the app and environment IDs. Read them from config or discover them from an existing page rather than hard-coding them.
  * Upload a minimal node: `guestParams.source` plus the IDs, with no cached SVGs and no `embeddedMacroContext` / `localId`. Verified on test page 458755: merfluence renders this correctly, and Confluence stores it as sent. Viewing the page does not write cached SVGs back.
  * **Never send cached SVGs with a changed `source`.** Verified: when `source` changes but the old `svgLight` / `svgDark` are kept, merfluence shows the old diagram. Simplest rule: always strip `svgLight`, `svgDark`, `renderedVersion` and `cacheV` on upload. Keeping them for unchanged diagrams is a possible later optimization.
  * `embeddedMacroContext` holds page-specific data (page ID, version, space, account). Don't send it on upload.

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
  * Files are only downloaded when output goes to a file (`rf fetch -o page.md` creates `page.assets/`). Printing to stdout doesn't download anything.
  * Filenames come from `__fileName` when present. Otherwise one extra call to the attachments API maps `fileId` -> filename (only for pages that have images).

Upload:

  * A local image file is uploaded as an attachment named after the file (attachment names are unique per page). If an attachment with that name already exists, a new version is uploaded only if the content changed. The `media` node then points at the attachment's `fileId`.
  * If the local file is missing but the page already has an attachment with that name, the existing attachment is reused. This makes "fetch to stdout, edit, upload" safe without downloading images.
  * A page must exist before files can be attached to it, so creating a new page with images is: create the page, upload attachments, update the body.
  * Attachments are uploaded with the v1 API (`POST /wiki/rest/api/content/{id}/child/attachment` for new files, `.../child/attachment/{attachment id}/data` for new versions; multipart, header `X-Atlassian-Token: no-check`). The response includes `extensions.fileId`, which goes into the `media` node.
  * A minimal node works: `mediaSingle` > `media` with `type: "file"`, `id` (the `fileId`) and `collection: "contentId-<page id>"`. No `width` / `height` or `__` attributes are needed.

Scope: images only for now. Other attachments (PDFs etc., shown as file cards in a `mediaGroup`) are fetched as plain links to the attachment.

### Confluence-only settings in markdown

Some Confluence settings have no markdown syntax. To keep round trips independent of the remote page, they are stored in the markdown file itself:

  * **Code blocks (including Mermaid):** in the fence info string, e.g. ```` ```mermaid theme=dark ````.
  * **Everything else (images, and later table column widths, captions, ...):** an `rf:` HTML comment directly after the element, on the same line:

    ```markdown
    ![roadmap](page.assets/roadmap.svg)<!-- rf: layout=center width=1070 -->
    ```

Rules:

  * Only non-default values are written. Defaults are what Confluence fills in when a setting is omitted, so a round trip without a comment is stable. For images that's left-aligned (`align-start`) at natural width with `widthType=pixel`; such an image has no comment.
  * The comment only applies when it's immediately adjacent to the element on the same line (comrak parses it as an inline HTML node following the image).
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
  * `<url>` (autolink) <-> `inlineCard`
  * `<url>` alone on a line plus an `rf:` comment <-> `embedCard` / `blockCard`, e.g. `<https://youtube.com/...><!-- rf: card=embed width=100 -->`

Upload:

  * Relative links to markdown files (`[setup](./setup.md)`) are rewritten to `https://<site>/wiki/spaces/<KEY>/pages/<id>`, using the target file's frontmatter `id` and `space_key`.
  * `upload --config` runs in two passes: first create/update every page so each one has an ID, then write the bodies with links resolved. This lets new pages link to each other.
  * A link to a markdown file that isn't in the upload set and has no page ID fails the upload with an error listing the unresolved links. A broken `setup.md` link on a Confluence page is worse than a failed upload.
  * Absolute Confluence URLs pass through unchanged.

Fetch:

  * Stdout output keeps absolute URLs. LLMs can follow them directly, since `rf fetch` accepts URLs.
  * `rf fetch -o <path>` scans the project's markdown files for frontmatter page IDs and rewrites matching page URLs to relative paths. The project root is the directory containing `.rfluence.yaml`, or the output directory if there is none. The scan only happens for file output, so the stdout path stays fast.
  * Recognised same-site URL forms: `/wiki/spaces/KEY/pages/ID` (with or without the trailing title) and tiny links (`/wiki/x/...`). Links are matched by page ID only; the title part is ignored (it changes when a page is renamed, and Confluence adds or removes it on save).

What Confluence does with links (verified on test page 295349):

  * Text links and smart links (`inlineCard`) created via the API to page URLs render and work as page links.
  * Confluence rewrites page URLs on save, inconsistently:
    * Without an anchor, a trailing title is removed: `/pages/131074/rfluence+image+API+test` is stored as `/pages/131074`.
    * With an anchor, the title is added: `/pages/131074#A:-...` is stored as `/pages/131074/rfluence+image+API+test#A:-...`.
  * So the stored URL form can't be predicted. `upload` writes `https://<site>/wiki/spaces/<KEY>/pages/<id>[#anchor]`, `fetch` resolves any form to the page ID, and round-trip comparison uses the resolved target, not the URL text.
  * Same-page anchor `href`s (`#...`) are stored exactly as sent.

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

Out of scope for now: links to non-markdown local files (`./spec.pdf`); see Future considerations.

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

After `rf upload` creates or updates a page, it writes the page's details (`id`, `version`, `url`, ...) back into the file's `rfluence:` frontmatter (see "Frontmatter"). No lock file is used.

Why:

  * `version` has to be updated after every upload for the conflict check to work, so the file is modified regardless. Keeping everything in frontmatter means one place that is always current.
  * `rf fetch` already produces frontmatter, so fetched and uploaded files have the same shape, and single-file and `--config` uploads work the same way.
  * Each file carries its own link to its page; nothing breaks if files are moved or copied without a side file.

Fallback when the link is lost (e.g. an LLM rewrites a file and drops the frontmatter):

  * Before creating a page, `rf upload` looks for an existing page with the same title in the target space. Confluence requires titles to be unique within a space, so this can't silently create a duplicate.
  * If a page is found, `rf` refuses to upload and reports the matching page ID, because without a `version` there is no way to check for remote edits. `--force` overwrites that page and writes the frontmatter back.

### `upload --config` mirrors the directory structure as the page tree

Each `.rfluence.yaml` entry's files are placed in Confluence following their directory structure under the entry's `ancestor`, rather than flat under the ancestor or via an explicit `parent` in each file. See "Page hierarchy" for the rules.

Why:

  * Doc folders (`docs/api/auth.md`, `docs/cli/auth.md`) keep their structure in Confluence instead of ending up side by side.
  * It needs nothing from the author. Explicit per-file parents are tedious and LLMs won't maintain them.
  * Directory nodes are found by title, consistent with the title fallback above, so no lock file is needed.
  * Moves are opt-in (`--move`) so `rf` doesn't silently undo reorganizations done in Confluence.

### Remote pages are only deleted with `--prune`, and only if `rf` created them

Deleting a local file never deletes its Confluence page by default. `upload --config` reports orphans (remote pages with no local file), and `--prune` moves them to the trash. Only pages and folders `rf` created, marked with an invisible `rfluence` content property, can be pruned. See "Renames and deletions".

Why:

  * Pages people added under an uploaded tree in Confluence must never be touched. A label could be removed by accident in the UI; a content property is invisible and can hold structured data.
  * The property records the version `rf` last uploaded, so `--prune` can skip pages edited in Confluence since then. Deleting a local file can't silently trash someone's changes.
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
  * `keyring` for token storage. With v3, enable the platform backend features explicitly (e.g. `sync-secret-service`, `apple-native`, `windows-native`); without them it silently uses an in-memory mock store.
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

Reference pages on the test site (personal space, ID 294914). Don't delete them:

  * [merfluence](https://tech-accounts11.atlassian.net/wiki/spaces/~71202005eb3148b0e1450da60d67073e3cc131/pages/295341) (ID 295341) has a diagram inserted through the Confluence editor, showing the full node the app writes, including cached SVGs and `embeddedMacroContext`.
  * [rfluence merfluence API test](https://tech-accounts11.atlassian.net/wiki/spaces/~71202005eb3148b0e1450da60d67073e3cc131/pages/458755) (ID 458755) has diagrams created through the API. A (source only) and B (source + default settings) render correctly; C (changed source + stale cached SVGs) shows the old diagram.
  * [rfluence image API test](https://tech-accounts11.atlassian.net/wiki/spaces/~71202005eb3148b0e1450da60d67073e3cc131/pages/131074) (ID 131074) has images created through the API: a minimal node, explicit size/layout, a new attachment version referenced by new and old `fileId`, and an external image. All render correctly.
  * [rfluence link API test](https://tech-accounts11.atlassian.net/wiki/spaces/~71202005eb3148b0e1450da60d67073e3cc131/pages/295349) (ID 295349) has page links (text link, URL with title, smart link) and headings with punctuation, unicode, duplicates and extra spaces for checking anchor IDs.
  * [rfluence tree test](https://tech-accounts11.atlassian.net/wiki/spaces/~71202005eb3148b0e1450da60d67073e3cc131/folder/131110) (folder ID 131110, under the space homepage) has nested folders and pages for checking hierarchy: folder `api`, page `api` (same title as the folder), folder `rfluence tree page` (same title as a page), and page `rfluence tree page` (moved from `api` to the top folder).
  * [rfluence label API test](https://tech-accounts11.atlassian.net/wiki/spaces/~71202005eb3148b0e1450da60d67073e3cc131/pages/524309) (ID 524309) has labels added via the API, including the results of splitting (`two`, `words`, `comma`, `label`), lowercasing (`upper`) and non-ASCII (`ünïcode`).
  * [rfluence lifecycle test](https://tech-accounts11.atlassian.net/wiki/spaces/~71202005eb3148b0e1450da60d67073e3cc131/folder/262167) (folder ID 262167, under the space homepage) has the results of the rename/delete tests: folder `new-name` holding a page with an `rfluence` content property (moved there from the trashed folder `old-name`), and pages moved up to this folder when their parent page / folder was deleted. A folder `old-name` and page `lifecycle parent page` were recreated after their originals were trashed, to show trashed titles can be reused. The originals are in the space trash.

## rf / rfluence

The command for retrieving Confluence pages. The intended target is to use it in AI skills when searching for documents.

### Auth

Env-based auth requires all three variables:

  * `CONFLUENCE_API_KEY` the API token
  * `CONFLUENCE_EMAIL` the Atlassian account email (Cloud API tokens authenticate with email + token over HTTP Basic, so the token alone is not enough)
  * `CONFLUENCE_BASE_URL` the site URL (e.g. `https://example.atlassian.net`)

Env-based auth takes precedence. If none of the variables are set, use the credentials stored by the `rf auth` command. If only some are set, exit with an error naming the missing variables rather than silently mixing env and stored credentials.

### Commands

  * `rf search <query>` used to search Confluence. Returns a list of matches with a description for the LLM to evaluate.
    * Free text maps to CQL `text ~ "<query>"`. Flags: `--space`, `--label`, `--limit`, and `--cql` for a raw CQL query.
    * Uses the v1 `/wiki/rest/api/search` endpoint (v2 has no search). Strip the `@@@hl@@@` highlight markers from excerpts.
    * Each result includes page ID, title, space, URL, last modified, labels, and excerpt (labels via `expand=content.metadata.labels`, no extra call).
  * `rf fetch <identifier>` used to fetch a specific page. Returns the page in markdown form. This could be used by an LLM, or by a user to download, modify, and re-upload. Includes frontmatter for where the page came from so that the upload command can easily upload this file back to Confluence.
    * Accepts a page ID, a full page URL, or `SPACE:Title` (titles are only unique within a space).
    * `--max-chars` and `--section <heading>` to avoid flooding an LLM's context with large pages.
    * `-o <path>` writes to a file and downloads images to `<name>.assets/` (see "Images and attachments").
  * `rf auth` walks a user through adding an auth token (base URL, email, token). The token should be saved to a keyring. If a keyring is not available, save it to `~/.config/rfluence/token` with restrictive permissions (0600). The token file is only used as a fallback when no keyring is available.
  * `rf upload <path>` uploads an MD file to Confluence using the frontmatter data.
    * If the remote page version is newer than the `version` in the frontmatter, refuse to upload unless `--force` is passed. This prevents overwriting edits made in Confluence.
    * A file without a page ID creates a new page; see "Frontmatter" > "New pages" (`--space` / `--parent` fill in missing values). If a page with that title already exists in the space, refuse unless `--force` is passed (see Decisions).
    * Frontmatter is stripped before upload. After a successful upload, the `rfluence:` block is written back (see "Frontmatter" > "Reading and writing").
    * `--dry-run` shows what would be created/updated without changing anything.
  * `rf upload --config <path>` uploads multiple pages using a config file. The config file (`.rfluence.yaml` in the project root) is a YAML list of entries, each mapping files (exact paths or globs) to a Confluence space and ancestor page, with optional labels. See "Upload config".
  * `rf diff <path>` shows the differences between a local file and the current remote page.

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
| `id` | string | `rf` | yes | Page ID. Absent means "create a new page" (subject to the title fallback in Decisions). |
| `space_key` | string | `rf`; user for new pages | yes | Space key. For files in `.rfluence.yaml`, must match the entry's `space_key`; a mismatch is an error (moving pages between spaces isn't supported). |
| `parent` | string | `rf`; user for new pages | single-file only | Parent page or folder ID. For files in `.rfluence.yaml`, the mirrored tree decides the parent and this is just a record. For single-file `rf upload`, a `parent` that differs from the page's current parent follows the `--move` rule (warn; `--move` moves it). |
| `title` | string | user; `rf` when needed | yes | Page title override. See "Title". |
| `version` | integer | `rf` | yes | Page version after the last fetch/upload. Upload refuses if the remote version is newer, unless `--force`. |
| `url` | string | `rf` | no | Page URL, for people and LLMs to follow. Ignored on upload. |
| `labels` | list of strings | `rf` and user | yes | Labels. Upload is additive (see "Labels"). |
| `weight` | integer | user only | `--config` only | Sibling order (see "Child page order"). Confluence has no equivalent, so `fetch` never writes it. |

  * IDs are strings (quoted in YAML) so they aren't parsed as numbers.
  * Unknown keys under `rfluence:` are an error, so typos are caught (same as `.rfluence.yaml`).
  * Content IDs are shared between pages and folders, so `parent` doesn't need a type.

#### Title

Confluence titles are separate from the page body, but LLM-written markdown usually starts with `# Title`. The rules keep both directions stable:

  * Upload: if `title` is not set and the body starts with an H1, that H1 is the page title and is removed from the body (so Confluence doesn't show the title twice). If `title` is set, it is the page title and the body is uploaded as-is, including any leading H1.
  * Fetch: if the page body doesn't start with an H1, write `# <page title>` at the top and no `title` key. If the body does start with an H1 (e.g. a different heading added in Confluence), write the `title` key so the next upload doesn't mistake that H1 for the title.
  * A file with neither `title` nor a leading H1 is an error for new pages.

#### Reading and writing

  * `rf fetch` (stdout and `-o`) writes `id`, `space_key`, `parent`, `version`, `url`, `labels`, and `title` when the Title rules require it.
  * After a successful upload, the `rfluence:` block is rewritten to exactly what `rf fetch` would produce for the page (including the page's full label set and the new version), plus the user-only `weight`. This keeps `fetch(upload(md))` and the written-back file identical.
  * `rf` only rewrites the `rfluence:` block. Other frontmatter keys, their order and comments are left byte-for-byte unchanged. Inside the block, keys are written in the order shown above, and comments in it are not preserved.
  * `rf fetch -o` over an existing file keeps that file's non-`rfluence` frontmatter keys and its `weight`, since Confluence doesn't store them.
  * Upload strips all frontmatter from the body. Non-`rfluence` keys are never sent to Confluence, so round-trip comparison covers the body and the `rfluence:` block only.

#### New pages

A file without `id` creates a page. It needs:

  * a space: `space_key`, the `.rfluence.yaml` entry, or `rf upload <path> --space <KEY>`;
  * a parent: `parent`, the mirrored tree, or `--parent <id>` (defaults to the space homepage if none is given);
  * a title: see "Title".

Flags only fill in missing values; they don't override frontmatter.

#### Relation to the content property

The `rfluence` content property on the page (see "Renames and deletions") records the version `rf` last uploaded. Right after an upload it equals the frontmatter `version`. If the remote page version is later higher than both, the page was edited in Confluence.

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
  * Precedence: for files covered by a config, the config tree decides the parent. Single-file `rf upload <path>` uses `parent` from the frontmatter (see "Frontmatter").

Verified on the test site (folder 131110):

  * Folders can be created via the API, nested in other folders, and pages can be created inside folders (`parentId` = folder ID).
  * Page titles are unique among pages in a space, and folder titles are unique among folders in a space, regardless of parent. A page and a folder **can** share a title.
  * Collisions return HTTP 400 `BAD_REQUEST` with titles `A page with this title already exists: ...` or `A folder exists with the same title in this space`.
  * A page can be moved by `PUT /wiki/api/v2/pages/{id}` with a new `parentId` and the next version number, without sending the body. The body is kept and the version is bumped.

### Child page order

Confluence keeps an order for sibling pages and folders. `upload --config` makes the order of `rf`-managed siblings match the local order. This costs extra API calls, which is acceptable: performance matters for reads (`fetch`, `search`), not for `upload --config`.

Local order:

  * Siblings are sorted by file or directory name using natural sort, case-insensitive, so numeric prefixes work as expected (`2-setup.md` before `10-faq.md`).
  * An optional `weight` (integer) in a file's `rfluence:` frontmatter overrides this: lower weights come first, ties are broken by name, and files without a weight come after weighted ones.
  * A directory's position comes from its `index.md` / `README.md` (`weight`, else the directory name). Directories that became folders sort by directory name.
  * `index.md` / `README.md` aren't siblings of the other files; they are the directory's page.

Applying it:

  * New pages and folders are placed in position when they're created: Confluence appends new children at the end, so `rf` creates the page and then moves it after its predecessor (one extra call).
  * Existing pages that are out of order: `upload` warns and leaves them, and `--move` reorders them. This is the same rule as parent changes (see "Page hierarchy"), so `rf` doesn't silently undo reordering done in Confluence. `--dry-run` lists the moves.
  * Only `rf`-managed siblings (those with the `rfluence` content property) are ordered among themselves. Pages people added are left where they are, so they may end up interleaved.
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

Verified on test page 524309:

  * Uppercase is lowercased (`UPPER` -> `upper`).
  * Spaces and commas silently **split** a label into several (`two words` -> `two` + `words`; `comma,label` -> `comma` + `label`). This is why `rf` must normalize spaces before sending.
  * Allowed: letters (including non-ASCII, e.g. `ünïcode`), digits, `-`, `_`, `/`.
  * Rejected with a generic HTTP 400 (`Could not add labels to content`, no reason given): `.`, `:`, `&`, `#`, `(`, `!`. Since the error doesn't say why, `rf` validates locally and reports the bad character itself.
  * Adding and removing labels leaves the page version unchanged.
  * New labels are searchable via CQL (`label = "dash-ok"`) immediately.

### Renames and deletions

Renaming or moving files:

  * Renaming a file has no effect on Confluence: the page ID is in the frontmatter and the title comes from frontmatter or the first H1, not the filename.
  * Moving a file to another directory changes its parent in the tree; handled by the `--move` rule in "Page hierarchy".
  * Changing a title (frontmatter `title` or H1) renames the page on upload, subject to the normal collision check. Links keep working because they use page IDs.
  * Links in other local files that point at a renamed file become unresolved, and `upload` fails with the list (see "Links").
  * Renaming a directory without an `index.md` renames its folder. The API can't rename folders (verified), and folders are found by title, so `rf` detects the rename through the directory's pages: their IDs are known, and if they all currently sit in an `rf`-managed folder with a different title, that is the old folder. With `--move`: create the new folder, move the pages into it, then trash the old folder once it's empty. Without `--move`: warn.

Deleting files:

  * Single-file `rf upload` never deletes anything.
  * `upload --config` lists the descendants of each entry's ancestor and reports orphans: `rf`-managed pages whose ID isn't in any local file's frontmatter, and `rf`-managed folders with no matching directory.
  * `--prune` moves orphans to the trash. `--dry-run` lists them first.
  * `--prune` skips (and reports) orphans that:
    * have been edited in Confluence since `rf` last uploaded them (page version != `version` in the `rfluence` property), unless `--force` is passed;
    * have child pages that aren't `rf`-managed, because deleting a page moves its children up a level (verified), which would silently reorganize pages people added.
  * Folders are only pruned once empty.

Content property (`rfluence`), written on every page and folder `rf` creates or updates:

```json
{ "managed": true, "version": 7, "path": "docs/api/auth.md", "config_labels": ["github", "ai-generated"] }
```

  * `version`: the page version after `rf`'s last upload. `path`: the local path relative to the project root (for reporting). `config_labels`: labels added from the `.rfluence.yaml` entry (see "Labels").
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
  * Results go to stdout, errors go to stderr, with distinct exit codes (e.g. not found, auth failure, version conflict).

## Testing

For testing, generate some MD pages that are indicative of what LLMs produce. Include Mermaid diagrams and the other features listed under "Markdown features LLMs commonly produce". For e2e testing these will need to be uploaded to a test space, then downloaded. Support recorded responses for faster testing with a way to enable live testing.

  * Most fidelity testing runs offline against `rfluence-convert`, using real ADF captured from Confluence as fixtures.
  * Recorded HTTP responses via `wiremock` or `rvcr`; set `RF_LIVE=1` to hit the real API. Strip auth headers from recordings.
  * Live e2e runs add a unique run ID to page titles (titles must be unique within a space) and clean up created pages afterwards. Be mindful of rate limits.

## Open questions

None currently.

## Suggested implementation order

  1. Build `rfluence-convert` and its test corpus.
  2. `rf fetch` and `rf search`.
  3. `rf upload` with version checks, then `rf upload --config`.

## Future considerations

Not required now; ideas to revisit later.

### `rf pull`: fetch a page tree into a directory

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

### rfluence-desktop

A testing application. The goal is to be able to fetch the Confluence page as markdown and view the rendered output. Use gpui-kit (verify the crate name; `gpui-component` is the known gpui component library).

  * Rendering Mermaid natively is the hard part; there's no mature Rust Mermaid renderer, so this likely needs a webview or shelling out to `mmdc`.
  * Keep it out of the workspace `default-members` so gpui compile times don't slow down CLI work.
  * `rf preview` (below) may cover the same need with much less effort.

### `rf preview`

Render markdown to an HTML page and open it in the browser. Mermaid diagrams are rendered by mermaid.js in the browser, so no Rust Mermaid renderer is needed. Could replace rfluence-desktop.

  * `rf preview <path>` renders a local MD file (comrak -> HTML, Mermaid blocks -> `<pre class="mermaid">`), writes it to a temp file, and opens it.
  * `rf preview <path> --roundtrip` runs the file through the converter offline (md -> ADF -> md) and shows the original and round-tripped output side by side with a text diff. Most useful for checking round-trip fidelity.
  * `rf preview <page> --remote` compares against Confluence's rendered HTML (`body-format=view`). Mermaid app diagrams likely won't appear there since apps render client-side, and `view` uses an older renderer that differs from the browser (e.g. heading anchor IDs), so `rf open <page>` (open the page URL) may be more practical.
  * Load mermaid.js from a CDN by default (optional `--offline` to embed it). Put `preview` behind a cargo feature so it doesn't add weight to the fast fetch/search path.
  * A `--watch` mode with live reload (`notify` + a small local server) would help while authoring.
