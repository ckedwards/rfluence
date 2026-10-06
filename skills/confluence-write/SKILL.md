---
name: confluence-write
description: Edit, create and publish Confluence pages as markdown with the rfluence command. Use when the user asks to update, edit or fix a Confluence page, create a new Confluence page, or publish or sync markdown docs to Confluence. For only searching or reading pages use confluence-read.
allowed-tools: Bash(rfluence fetch *) Bash(rfluence check *) Bash(rfluence search *)
---

# Editing and publishing Confluence pages with rfluence

`rfluence` turns Confluence pages into markdown files and uploads them back. A page fetched, edited and uploaded keeps everything markdown can't show, through markers in the file.

## Rules

- **Uploading publishes to a site other people read.** Before any `rfluence upload` without `--dry-run`, run it with `--dry-run`, show the user what it would change, and get their go-ahead, unless they already told you to publish.
- **Synced blocks are read-only. Never edit them, and never remove their markers.** Content between `<!-- rf: synced-block … read-only -->` and `<!-- rf: end-synced-block -->` is shared with other pages, and Confluence only updates those copies when it's edited in Confluence's editor. If the user's change touches that content, leave it as it is and tell them: it has to be edited in Confluence, on the page named by the marker's `page=` (or, without `page=`, this page). Don't get around it by deleting or changing the markers: that removes the synced block from the page, and its copies on other pages can stop showing its content.
- **Never use `--force` without asking.** It overwrites changes other people made in Confluence, uploads approximations of content Confluence can't store, or uploads to an existing page that has the new page's title.
- Never print or handle API tokens. If rfluence says you're not logged in, ask the user to run `rfluence auth login`.

## Edit an existing page

1. **Fetch it to a file**, in the round-trip form (not `--simplified`, which can't be uploaded):

   ```shell
   rfluence fetch 123456 -o docs/release-process.md
   ```

   This writes the markdown and downloads its images to `docs/release-process.assets/`. Exit 6 means the file already exists with local changes or holds another page: ask the user before adding `--force`, which overwrites it.

2. **Edit the markdown.** Keep what rfluence put there:
   - the `rfluence:` block in the frontmatter (`id`, `version`, `url`, ...): it links the file to the page; change only `labels` (and `title`, to rename);
   - every `<!-- rf: ... -->` comment, where it is: they hold Confluence settings and structure (layouts, tabs, card types, widths);
   - ```` ```adf ```` blocks: Confluence content markdown can't express; move or delete one whole, never edit inside;
   - synced blocks: don't touch them at all (see Rules).

   See [markdown.md](markdown.md) for how markdown maps to Confluence, and what the markers mean.

3. **Check it**:

   ```shell
   rfluence check docs/release-process.md
   ```

   Errors (exit 1) are markdown Confluence can't store, such as footnotes or raw HTML: rewrite them. Warnings are uploaded as the closest Confluence equivalent; mention them to the user if they matter.

4. **Upload**, dry run first:

   ```shell
   rfluence upload docs/release-process.md --dry-run
   rfluence upload docs/release-process.md
   ```

   The upload attaches new or changed images, keeps inline comments on their text (and reports any it detaches), and writes the new version into the file's frontmatter.

## Create a page

Write a markdown file whose first line is the title as a heading:

```markdown
---
rfluence:
  space_key: ENG
  parent: "123456"
  labels: [runbook]
---

# Rotating the API keys

...
```

`space_key` and `parent` (a page or folder ID; default: the space's homepage) can also be given as `--space` and `--parent`. Then `rfluence check`, `rfluence upload --dry-run`, and `rfluence upload`. Afterwards the file's frontmatter has the new page's `id`: keep it, so the next upload updates that page.

If the space already has a page with that title, upload refuses (exit 6) and names the page: that page probably is this file's page (fetch it and edit that instead), or choose another title.

## Many files: `upload --config`

A project can map directories of markdown files to page trees with a `.rfluence.yaml` config. `rfluence upload --config` then creates and updates the whole tree: directories with an `index.md` or `README.md` become pages, others folders. Always run `rfluence upload --config --dry-run` first and show the plan. Use `--move` (move pages to match the tree), `--prune` (trash pages whose files are gone) and `--prune-labels` only when the user asks for them.

## When something fails

| Exit | Meaning | What to do |
| --- | --- | --- |
| 1 | Content Confluence can't store (or, with `--warnings-are-errors`, approximations) | Fix the lines `rfluence check` reports |
| 2 | Bad file or arguments: unknown frontmatter keys, a link to a markdown file without a page, a missing image, a changed synced block, not logged in, no site | Fix what the message says; links need the target file to have an `id` (or be in the same `upload --config`). For "the content of synced block … was changed": put the original text back (or leave nothing between its markers) and tell the user to make that change in Confluence |
| 3 | Page not found (or in the trash) | Check the `id`; don't create a replacement without asking |
| 4 | Token not accepted, or no permission for this page or space | Tell the user (`rfluence auth login` replaces the token; permissions are set in Confluence) |
| 5 | Confluence or the network failed, after retries | Report it. For `upload --config`, running the same command again continues where it stopped |
| 6 | The page changed in Confluence since the file was fetched, a file without an `id` whose title is taken, or `fetch -o` over local changes | Don't `--force`. Fetch the current page to a new file, carry the user's edits over to it, and upload that |
