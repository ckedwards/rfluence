# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/ckedwards/rfluence/releases/tag/v0.1.0) - 2026-10-06

The first release.

### Added

- `rfluence fetch`: a Confluence page as markdown, by page ID, URL (including tiny links) or `SPACE:Title`. The round-trip form keeps everything markdown can't show (macros, layouts, settings) so it can be uploaded back unchanged; `--simplified` is a shorter form for reading. `--section` and `--max-chars` for long pages, `--json`, and `-o` to write the page and its images to files.
- `rfluence search`: free text, optionally in spaces and with labels, or a raw CQL query (`--cql`).
- `rfluence check`: reports, with line numbers, markdown that Confluence can't store exactly (warnings for close equivalents, errors for content it can't store); `--warnings-are-errors`.
- `rfluence upload`: updates a page from its markdown file, or creates one. It refuses to overwrite changes made in Confluence since the file was fetched (`--force` overrides), attaches local images, turns links between markdown files into page links, keeps inline comments on their text, and adds labels (`--prune-labels` removes them). `--dry-run`, `--move`, `--space` and `--parent`.
- `rfluence upload --config`: publishes a directory as a page tree from `.rfluence.yaml`, with folders, page order, `--move`, and `--prune` to trash pages whose files are gone.
- Markdown ↔ ADF conversion covering GitHub-flavored markdown, panels (`> [!NOTE]`), expands, tables (HTML tables with markdown inside when needed), images, emoji, status and mentions, page layouts, tabs, and synced blocks (read-only).
- Mermaid diagrams: uploaded for merfluence or Mermaid Diagrams Viewer, whichever the site has, otherwise as code blocks.
- `rfluence auth login`, `logout`, `status` and `token`: one Atlassian account (email and API token, kept in the system keyring, or in a file where there's no keyring), or `CONFLUENCE_BASE_URL`, `CONFLUENCE_EMAIL` and `CONFLUENCE_API_KEY` in the environment. Rejected tokens are detected and explained.
- `rfluence config set default-site`: the site used when a command doesn't name one.
- Retries for rate limits and temporary server errors, timeouts, and exit codes by kind of failure (content, usage, not found, auth, API or network, would lose changes).
- Agent Skills for AI agents: `confluence-read` and `confluence-write`.
