---
name: confluence-read
description: Search and read Confluence pages with the rfluence command. Use when the user asks what internal documentation, a wiki page, runbook, design doc or Confluence space says, gives a Confluence page URL, or asks you to find, read, summarize or answer questions from Confluence. Read-only; for editing or publishing pages use confluence-write.
allowed-tools: Bash(rfluence search *) Bash(rfluence fetch *)
---

# Reading Confluence with rfluence

`rfluence` reads Confluence Cloud pages as markdown. Searching and reading change nothing in Confluence.

## Workflow

1. **Search** unless the user gave a page URL or ID:

   ```shell
   rfluence search "rate limit"                       # free text, pages only, 10 results
   rfluence search "rate limit" --space ENG --limit 20
   rfluence search "deploy" --label runbook --label prod  # labels: all of them
   rfluence search --cql 'title ~ "onboarding" and lastmodified > now("-30d")'
   ```

   Each result has the page ID, title, space, last update, labels, URL and an excerpt. Pick pages by title and excerpt. If nothing fits, try other words (synonyms, a product name, fewer words) before telling the user there's nothing. "No results." is not an error.

2. **Read** with `--simplified`, the short form meant for reading:

   ```shell
   rfluence fetch 123456 --simplified
   rfluence fetch "https://example.atlassian.net/wiki/spaces/ENG/pages/123456/Title" --simplified
   rfluence fetch "ENG:Release process" --simplified    # SPACE:Title
   ```

   For long pages, don't read everything at once:

   ```shell
   rfluence fetch 123456 --simplified --max-chars 20000    # cut, with the list of sections
   rfluence fetch 123456 --simplified --section "Rollback" # one section (heading text or #anchor)
   ```

3. **Follow links** to other pages the same way: page links in the output are full URLs, which `rfluence fetch` accepts.

4. **Answer with sources**: name the pages you used, with their URLs (the `url` in each page's frontmatter).

## What the output looks like

The frontmatter has the page's `title`, `url`, `space_key`, `labels`, when it was last `updated`, and `simplified: true`. The body is markdown. Confluence-only things are shown as text:

- statuses, mentions and dates as their text: `[IN PROGRESS]`, `@Chris Edwards`, `2027-01-01`;
- macros without text as one-line markers: `[Table of contents]`, `[Child pages]`;
- images as `[image: alt text]` (the file isn't downloaded);
- expandable sections as a **bold title** followed by their content;
- tabs as each tab's **bold title** followed by its content;
- a synced block copy whose source can't be read as `[Synced block from page 123: not available]`;
- Mermaid diagrams as ```` ```mermaid ```` source.

## When something fails

| Exit | Meaning | What to do |
| --- | --- | --- |
| 2 | Not logged in, no site to use, or a bad argument (the message says which) | Ask the user to run `rfluence auth login` (it needs their API token; never ask for or handle the token yourself), or pass `--site <site>` |
| 3 | No such page, or no such `--section` (the message lists the page's sections) | Check the ID or URL; search for it; pick a listed section |
| 4 | The token isn't accepted (expired, revoked), or the account may not see this page | Tell the user; `rfluence auth login` replaces the token |
| 5 | Confluence or the network failed, after retries | Report it; don't loop on it |

Messages go to stderr. Lines like `rfluence: Confluence is busy (HTTP 429); trying again in 2 s (1/3)` are retries in progress, not failures.

## Don'ts

- Don't use `rfluence fetch -o`, `rfluence upload` or `--force` here: changing pages is confluence-write's job, and needs the user's go-ahead.
- Don't print or repeat API tokens (`rfluence auth token` prints one).
