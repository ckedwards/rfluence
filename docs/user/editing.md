# Editing and creating pages

## Edit a page

```shell
rfluence fetch 123456 -o docs/release.md     # 1. the page, and its images in docs/release.assets/
$EDITOR docs/release.md                      # 2. edit
rfluence check docs/release.md               # 3. anything Confluence can't store?
rfluence upload docs/release.md --dry-run    # 4. what would change
rfluence upload docs/release.md              # 5. upload
```

While editing:

- **Keep the `rfluence:` frontmatter.** It links the file to its page (`id`) and records the version you fetched. You can change `labels`.
- **Keep the `<!-- rf: … -->` comments and ```` ```adf ```` blocks.** They hold layouts, tabs, macros and settings that markdown can't show. Move or delete them whole; don't edit inside `adf` blocks.
- **Leave synced blocks alone.** Content marked `read-only` is shared with other pages and can only be edited in Confluence.

See [Markdown in Confluence](markdown.md) for what maps to what.

What upload does:

- uploads new and changed images as attachments, reusing unchanged ones;
- turns links to other markdown files (`[setup](./setup.md#install)`) into links to their pages;
- keeps inline comments on their text, and tells you about any whose text you changed or removed;
- adds the file's labels (it never removes labels unless you add `--prune-labels`);
- writes the new version back to the file, so you can keep editing and uploading.

If nothing changed, nothing is uploaded.

## When the page changed in Confluence

If someone edited the page in Confluence after you fetched it, upload refuses (exit code 6) rather than overwrite their changes. Fetch the current page to another file, carry your changes over, and upload that. `--force` overwrites their changes; use it only if you mean to.

## Create a page

Write a markdown file whose first line is the title:

```markdown
---
rfluence:
  space_key: ENG
  parent: "123000"
  labels: [runbook]
---

# Rotating the API keys

...
```

```shell
rfluence upload rotating-keys.md --dry-run
rfluence upload rotating-keys.md
```

`space_key` and `parent` can also be given as `--space ENG --parent 123000`; without a parent, the page goes under the space's homepage. After the upload, the file's frontmatter has the new page's `id`, so the next upload updates it.

If the space already has a page with that title, upload refuses and names it, so you don't get a duplicate. If that page is this file's page (say the `id` was lost), add its `id` to the frontmatter.

## Moving a page

Set `parent` in the frontmatter to another page or folder ID and upload with `--move`. Without `--move`, upload only warns that the page is somewhere else.

## Many pages

To keep a directory of docs in Confluence as a page tree, see [Publishing a docs tree](publishing-a-docs-tree.md).
