# Markdown in Confluence (rfluence)

GitHub-flavored markdown, with these mappings. `rfluence check <file>` reports anything that won't upload exactly.

## Maps directly

| Markdown | Confluence |
| --- | --- |
| `# Title` as the first line | the page title (not repeated in the body) |
| headings, paragraphs, bold, italic, ~~strike~~, `code`, lists, task lists (`- [ ]`) | the same |
| fenced code blocks with a language | code blocks |
| ```` ```mermaid ```` | a Mermaid diagram (if the site has a Mermaid app; otherwise a code block) |
| `> [!NOTE]`, `[!IMPORTANT]`, `[!TIP]`, `[!WARNING]`, `[!CAUTION]` | info, note, success, warning, error panels |
| `<details><summary>Title</summary>` (blank line, content, blank line) `</details>` | an expand (at the top level or in a layout column) |
| GFM tables | tables; tables with lists, code or merged cells are HTML tables with markdown in their cells |
| `![alt](page.assets/diagram.png)` on its own line | an attached image (uploaded from the local file) |
| `[text](https://…)` | a link; `<https://…>` alone is a smart link (card) |
| `[text](./other-page.md#a-heading)` | a link to that file's page and heading (the file needs an `id`) |
| emoji characters or `:shortcodes:` | emoji |
| `<u>`, `<sub>`, `<sup>`, `<span style="color: …">` | underline, subscript, superscript, text colour |
| `<kbd>`, `<code>`, `<b>`, `<i>`, `<s>`, `<br>` | the closest equivalent: inline code, bold, italic, strike, a line break (a warning) |
| other inline HTML (`<abbr>`, `<span class=…>`, …) | dropped, text kept (a warning) |

## Doesn't upload exactly (errors)

Footnotes, raw HTML blocks (`<div>`, …), images inside a sentence, quotes inside quotes, a table inside a table. Rewrite these: for example, a footnote as a parenthesis or a "Notes" section, an inline image as its own paragraph.

## Markers (keep them as they are)

`<!-- rf: ... -->` comments hold what markdown can't. Markdown viewers don't show them. Keep them, and keep content between opening and closing markers together.

| Marker | What it is |
| --- | --- |
| `<!-- rf: columns=50,50 -->` … `<!-- rf: column -->` … `<!-- rf: end-columns -->` | a layout: each column's content between the markers (only at the top level) |
| `<!-- rf: tabs -->`, `<!-- rf: tab title="Install" -->` … `<!-- rf: end-tabs -->` | tabs: each tab's content after its `tab` marker. Editing tab content and titles is fine |
| `<!-- rf: synced-block id=… read-only -->` … `<!-- rf: end-synced-block -->` | a synced block, shared with other pages. **Read-only**: don't edit its content and never remove the markers (that deletes the block, and its copies on other pages can stop showing it); changes are made in Confluence. Content left as it is, or nothing between the markers, keeps it unchanged |
| `<!-- rf: synced-block id=… page=… read-only -->` | a copy of a synced block from another page: same rule |
| a comment at the end of a line or after a table or image, e.g. `<!-- rf: layout=center width=400 -->`, `<!-- rf: align=center -->`, `<!-- rf: card=inline -->` | settings of that paragraph, heading, table, image or link |

Other things you'll see:

- ```` ```adf ```` blocks hold Confluence content markdown can't express (macros, custom panels). Keep them whole; don't edit the JSON.
- `<span data-adf='…'>IN PROGRESS</span>` is an inline Confluence element (a status, mention, date). Keep the span; its visible text is display-only.
- `[Title](https://…)<!-- rf: card=inline -->` is a smart link: Confluence shows the target page's current title, so the text doesn't matter.

## Frontmatter

```yaml
---
rfluence:
  id: "123456"          # the page; absent for a new page
  space_key: ENG
  parent: "123000"      # the parent page or folder
  title: Page title     # only when the body doesn't start with the title as `# Title`
  version: 7            # the version the file was fetched or uploaded at
  url: https://example.atlassian.net/wiki/spaces/ENG/pages/123456
  labels: [runbook, ai-generated]
---
```

Other frontmatter keys (outside `rfluence:`) are kept and never uploaded. Unknown keys inside `rfluence:` are an error. Labels are only added on upload; `--prune-labels` also removes labels the file doesn't list.
