# rfluence

**Confluence pages as markdown, both ways.** `rfluence` is a command-line tool that fetches Confluence Cloud pages as markdown files, and uploads markdown files back as pages, without losing what markdown can't show.

## Why

Docs increasingly get written and read by people and AI agents working in plain text: in editors, in git, through LLM tools. Confluence stores pages in its own rich format (ADF), and most tools that bridge the two go one way, or lose content on the way back: a macro, a layout, a panel, an inline comment.

rfluence treats markdown as a real way to work on Confluence pages:

- **Read**: search a site and get any page as compact markdown an LLM can use.
- **Edit**: fetch a page to a file, change it with any editor or agent, and upload it again. Everything markdown can't express (layouts, macros, settings, comments) survives in markers in the file.
- **Publish**: keep docs in a repository and upload a whole directory as a page tree.

**North star: content round-trips.** Fetch a page, upload the markdown, fetch it again: you get the same markdown. Upload markdown and fetch it back: you get the same markdown (tidied into one consistent form). When something can't be represented, rfluence says so (`rfluence check`) instead of silently changing the page.

## Install

rfluence is on [crates.io](https://crates.io/crates/rfluence); prebuilt binaries for Linux, macOS and Windows will be on the [releases page](https://github.com/ckedwards/rfluence/releases) from the next release. With Rust 1.88 or later:

```shell
cargo install rfluence                                         # the latest release
cargo install --git https://github.com/ckedwards/rfluence rfluence   # the latest source
```

See [Installation](docs/user/installation.md).

## Log in

rfluence uses your Atlassian account's email and an [API token](https://id.atlassian.com/manage-profile/security/api-tokens):

```shell
rfluence auth login                              # asks for your site, email and token
rfluence config set default-site example         # example.atlassian.net, used when a command names no site
rfluence auth status                             # check it works
```

The token is kept in your system keyring. For CI, set `CONFLUENCE_BASE_URL`, `CONFLUENCE_EMAIL` and `CONFLUENCE_API_KEY` instead. See [Authentication](docs/user/authentication.md).

## Use it

```shell
# Find and read pages
rfluence search "release process" --space ENG
rfluence fetch 123456 --simplified               # compact markdown for reading
rfluence fetch https://example.atlassian.net/wiki/spaces/ENG/pages/123456

# Edit a page
rfluence fetch 123456 -o docs/release.md          # the page and its images
$EDITOR docs/release.md
rfluence check docs/release.md                    # anything Confluence can't store?
rfluence upload docs/release.md --dry-run         # what would change
rfluence upload docs/release.md

# Create a page: a markdown file starting with "# Title"
rfluence upload new-page.md --space ENG --parent 123000

# Publish a directory as a page tree, configured in .rfluence.yaml
rfluence upload --config --dry-run
```

Uploads refuse to overwrite changes made in Confluence since you fetched a page, and never delete anything unless asked.

> [!IMPORTANT]
> **Mermaid diagrams.** LLMs draw diagrams as ```` ```mermaid ```` code blocks, but Confluence can't show those by itself: your site needs a Mermaid app ([merfluence](https://github.com/edlopez000/merfluence) or [Mermaid Diagrams Viewer](https://marketplace.atlassian.com/apps/1232887/mermaid-diagrams-viewer)). rfluence detects which one is installed and uploads diagrams for it; without one, they stay code blocks. See [Mermaid diagrams](docs/user/mermaid.md).

## Documentation

- [Installation](docs/user/installation.md)
- [Authentication](docs/user/authentication.md)
- [Reading pages](docs/user/reading.md): search, fetch, long pages
- [Editing and creating pages](docs/user/editing.md): fetch, edit, check, upload
- [Publishing a docs tree](docs/user/publishing-a-docs-tree.md): `upload --config` and `.rfluence.yaml`
- [Markdown in Confluence](docs/user/markdown.md): what maps to what, and the `rf:` markers
- [Mermaid diagrams](docs/user/mermaid.md): which apps draw them, and how rfluence uploads them
- [Using rfluence with AI agents](docs/user/ai-agents.md): skills for Claude Code and other agents
- [Troubleshooting](docs/user/troubleshooting.md): exit codes and common errors

Working on rfluence itself: [development.md](development.md) (building, testing) and [design.md](design.md) (how it works, and why).

## License

[MIT](LICENSE)
