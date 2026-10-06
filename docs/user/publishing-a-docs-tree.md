# Publishing a docs tree

`rfluence upload --config` uploads a directory of markdown files as a Confluence page tree, mirroring the directories. It's meant for docs kept in a repository.

## The config file

Put a `.rfluence.yaml` in the project root. It's a list of entries, each mapping files to a place in Confluence:

```yaml
- globs:
    - docs/**/*.md
  exclude:
    - docs/drafts/**
  space_key: ENG
  ancestor_id: "123000"        # the page (or folder) the tree goes under
  labels: [ai-generated]       # added to every page in this entry
```

| Field | |
| --- | --- |
| `globs` / `paths` | which files (at least one of them). Globs use `**` for any depth; files ignored by `.gitignore` are skipped |
| `exclude` | globs to leave out |
| `space_key` | the space |
| `ancestor_id` or `ancestor` | the parent page or folder, by ID or by title |
| `labels` | labels added to every page in the entry |
| `root` | the directory that maps to the ancestor (by default, the common directory of the globs) |
| `folder_title` | titles for folders, e.g. `"{parent} / {dir}"` (default `{dir}`), when directory names would clash |

## How files become pages

```
docs/
  README.md          ->  page "GitHub How-tos" (the directory's page), under the ancestor
  01-setup.md        ->    page "Setting up GitHub"
  02-workflow.md     ->    page "Our GitHub workflow"
  actions/           ->    folder "actions" (no README.md or index.md)
    runners.md       ->      page "Self-hosted runners"
```

- Page titles come from each file's first `# Heading` (or `title:` in its `rfluence:` frontmatter).
- A directory with an `index.md` or `README.md` becomes that page; other directories become folders.
- Pages are ordered by file name, with numbers in order (`2-setup` before `10-faq`). A `weight:` number in a file's `rfluence:` frontmatter overrides that: lower weights first, and pages with a weight before those without.
- Files without a page get one, and their `id` is written into them.
- Links between the files become links between the pages.

## Running it

```shell
rfluence upload --config --dry-run    # the plan: the tree, and what would be created or updated
rfluence upload --config
```

Every file is checked before anything is sent. If an upload stops part-way (a network error, say), run the same command again to continue.

Options:

- `--move`: move pages that were moved or reordered in Confluence (or whose files moved) back to where the config puts them. Without it, rfluence only warns.
- `--prune`: trash pages and folders that rfluence created whose files are gone. It skips pages someone edited in Confluence since, and anything with other pages under it. Add `--dry-run` to see what it would trash.
- `--prune-labels`: remove labels the files (and config) no longer list.

Without these options rfluence never moves existing pages or deletes anything (new pages are placed in order). Pages other people add under the ancestor are left alone, and `--prune` only ever trashes pages and folders rfluence created.
