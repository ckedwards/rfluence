# Markdown in Confluence

rfluence uses GitHub-flavored markdown. `rfluence check <file>` tells you about anything that won't upload exactly, with line numbers: **warnings** are uploaded as the closest Confluence equivalent, **errors** can't be stored and stop the upload (unless `--force`).

## What maps to what

| Markdown | Confluence |
| --- | --- |
| `# Title` on the first line | the page title |
| headings, bold, italic, ~~strike~~, `inline code`, lists, `- [ ]` task lists | the same |
| code blocks with a language | code blocks |
| ```` ```mermaid ```` | a Mermaid diagram, if the site has a Mermaid app; otherwise a code block (see [Mermaid diagrams](mermaid.md)) |
| `> [!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, `[!CAUTION]` | info, success, note, warning and error panels |
| `<details><summary>Title</summary>` … `</details>` | an expand |
| tables | tables; ones with lists, code or merged cells are written as HTML tables with markdown inside |
| `![alt](images/diagram.png)` on its own line | an image, uploaded as an attachment |
| `[text](https://…)` | a link; `<https://…>` on its own is a smart link |
| `[text](./other.md#heading)` | a link to the other file's page (it needs a page: an `id`, or the same `upload --config`) |
| emoji, or `:shortcodes:` | emoji |
| `<u>`, `<sub>`, `<sup>`, `<span style="color: …">` | underline, subscript, superscript, coloured text |

Errors (rewrite these): footnotes, raw HTML blocks such as `<div>`, an image in the middle of a sentence, a quote inside a quote, a table inside a table.

## What you'll see in fetched pages

Confluence has things markdown doesn't. In the round-trip form they're kept in ways markdown viewers hide or show plainly, so the file can be uploaded back:

- **`<!-- rf: … -->` comments** hold settings and structure: image sizes, table widths, `<!-- rf: columns=50,50 -->` … `<!-- rf: end-columns -->` around page layouts, `<!-- rf: tabs -->` around tabs, `<!-- rf: card=inline -->` after smart links. Keep them where they are.
- **```` ```adf ```` blocks** hold content with no markdown form, such as macros. Keep them whole; don't edit inside.
- **`<span data-adf='…'>text</span>`** is an inline element like a status or a mention. Keep the span.
- **Synced blocks**, between `<!-- rf: synced-block … read-only -->` and `<!-- rf: end-synced-block -->`, are shared with other pages and can only be edited in Confluence. Leave them as they are.
- **Frontmatter**: the `rfluence:` block links the file to its page:

  ```yaml
  ---
  rfluence:
    id: "123456"
    space_key: ENG
    parent: "123000"
    version: 7
    url: https://example.atlassian.net/wiki/spaces/ENG/pages/123456
    labels: [runbook]
  ---
  ```

  Other frontmatter keys are yours: kept, and never uploaded.

Comments people leave on the page's text (inline comments) aren't in the markdown; upload keeps them on their text.
