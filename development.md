# Developing rfluence

How to build, run and test rfluence. What it does and why is in [design.md](design.md).

## Prerequisites

  * Rust (stable), via `rustup`. On Arch / Omarchy: `sudo pacman -S rustup && rustup default stable`.
  * For the fixture scripts: `curl`, `jq`, `python3`.

No system libraries are needed: HTTPS uses rustls, and the Linux keyring store is pure Rust.

## Workspace

```
crates/
  rfluence-convert   pure markdown <-> ADF conversion (no network or files); most tests live here
  rfluence-client    Confluence API client and credentials
  rfluence-cli       the `rfluence` command (published as the `rfluence` crate)
fixtures/
  confluence/        captured Confluence pages: ADF, metadata, attachments, reference markdown
  markdown/          LLM-style markdown corpus for round-trip tests
scripts/             capture and restore the reference pages
```

## Build and run

From the repo, rebuilding as needed (everything after `--` goes to `rfluence`):

```shell
cargo run -q -p rfluence -- fetch 458790
```

Or build once and run the binary:

```shell
cargo build --release
./target/release/rfluence fetch 458790
```

To try upload, fetch a page to a file, edit it, and send it back (`--dry-run` first shows what would change). Use a page in the test space (`rfluencete`) that isn't one of the reference pages, since those are fixtures:

```shell
./target/release/rfluence fetch <id> -o scratch/page.md
./target/release/rfluence upload scratch/page.md --dry-run
./target/release/rfluence upload scratch/page.md
```

Or install it, so `rfluence` works anywhere:

```shell
cargo install --path crates/rfluence-cli
```

This installs to `~/.cargo/bin`; if that isn't on your `PATH`, add `export PATH="$HOME/.cargo/bin:$PATH"` to your shell profile. Re-run the install after changing the code.

## Credentials

rfluence keeps one account (your email and API token, which works on every site your account is on), like `gh auth`, and a default site: the Confluence site commands use when none is named.

```shell
rfluence auth login                             # prompts for your email and API token
rfluence config set default-site tech-accounts11 # https://tech-accounts11.atlassian.net (checks the token there)
rfluence auth status                            # the account, and whether the token works on the default site
rfluence auth token                             # print the token in use
rfluence auth logout
```

A command uses a page URL's own site, else `--site`, else `RFLUENCE_SITE`, else the default site. For scripts, `--with-token` reads the token from standard input:

```shell
printf '%s' "$TOKEN" | rfluence auth login --site example --email me@example.com --with-token
```

Or set all three `CONFLUENCE_*` variables, which are used for their site. The repo's `.env` (gitignored) has them for the test site:

```shell
set -a; . ./.env; set +a
```

Create API tokens at <https://id.atlassian.com/manage-profile/security/api-tokens>.

`RFLUENCE_CONFIG_DIR=<dir>` keeps accounts somewhere other than `~/.config/rfluence`, and `RFLUENCE_NO_KEYRING=1` stores tokens in files instead of the system keyring; the tests use both so they never touch your real accounts. `RFLUENCE_RETRY_UNIT_MS=1` makes retry waits (1, 2, 4 s, and `Retry-After`) milliseconds instead of seconds, for tests that make Confluence (or the fake) fail on purpose. `RFLUENCE_PROMPTS_FROM_STDIN=1` makes `auth login` read its answers (the token too) from standard input instead of the terminal, so tests can drive its prompts.

## Tests

```shell
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all               # CI fails on unformatted code
```

Most tests run offline against captured Confluence responses (`fixtures/confluence`) and the markdown corpus (`fixtures/markdown`):

| Tests | What they check |
| --- | --- |
| `rfluence-convert` unit tests | The building blocks; the normalizer on the corpus (fixed point, unchanged structure) |
| `rfluence-convert/tests/fetch.rs` | Each captured page's markdown matches its `page.md` / `page.simplified.md`; editor saves don't change the markdown |
| `rfluence-convert/tests/roundtrip.rs` | `normalize(md) == fetch(upload(md))` on the corpus; fetch -> upload -> fetch on every captured page; `check` diagnostics |
| `rfluence-convert/tests/upload.rs` | Snapshots of the ADF upload produces for the corpus |
| `rfluence-client/tests/api.rs` | The client against recorded responses on a mock server |
| `rfluence-cli/tests/*.rs` | The `rfluence` binary: `fetch`, `search`, `upload`, `check` and `auth` (two mock sites) output and exit codes; `upload.rs` checks the requests upload sends |
| `rfluence-cli/tests/upload_tree.rs` | `upload --config` against an in-memory Confluence (`tests/fake/`), which keeps pages, folders, labels and properties between requests |

### Updating expected output

After an intended change to what fetch writes, rewrite the reference markdown and review the diff:

```shell
RFLUENCE_UPDATE_FIXTURES=1 cargo test --workspace
git diff fixtures/confluence/*/page*.md
```

The upload snapshots (`crates/rfluence-convert/tests/snapshots/`) use [insta](https://insta.rs): review changes with `cargo insta review` (`cargo install cargo-insta`), or accept them all with `INSTA_UPDATE=always cargo test --workspace` and review the diff.

### Live tests

`rfluence-cli/tests/live.rs` fetches the reference pages from the real test site, checks the output still matches the fixtures, and times `rfluence fetch`, `search` and `upload`. The upload test creates a temporary page in `rfluencete` (with an image, a label and links), updates it, checks that fetching it gives the file back, and trashes it. The `upload --config` test uploads a small tree (pages and a folder) under a temporary page twice, then trashes all of it. They're skipped unless `RFLUENCE_LIVE` is set; credentials come from the environment or `.env`:

```shell
RFLUENCE_LIVE=1 cargo test -p rfluence --test live -- --nocapture
```

## Looking at conversions

Each captured page has its markdown next to it: `fixtures/confluence/<page>/page.md` (round-trip form) and `page.simplified.md` (`--simplified`). Images point at the folder's `attachments/`, so they preview with images.

Two examples in `rfluence-convert` help when working on the converter:

```shell
# A captured page as markdown
cargo run -q -p rfluence-convert --example fetch -- fixtures/confluence/emoji

# The normalized form of a markdown file (or stdin)
cargo run -q -p rfluence-convert --example norm -- fixtures/markdown/tables.md
```

## Reference pages and fixtures

The captured pages, how they were made, and what each covers are described in [fixtures/confluence/README.md](fixtures/confluence/README.md), along with:

  * `scripts/capture-fixtures.sh`: re-capture a page, or add one (`--new <name> <page id>`)
  * `scripts/restore-reference-pages.py`: recreate the pages in another space or site

## CI and releases

GitHub Actions run `.github/workflows/ci.yaml` on every push to `master` and every pull request: build and tests on Linux, macOS and Windows, clippy (warnings are errors), `cargo fmt --check`, and a build with the minimum Rust version (1.88). The live tests skip themselves there.

Releases are made by [release-plz](https://release-plz.dev), from `.github/workflows/release.yaml` and `release-plz.toml`:

1. On every push to `master`, release-plz opens (or updates) a release PR. It bumps the shared version in the root `Cargo.toml` (`[workspace.package]` and the two `rfluence-*` entries in `[workspace.dependencies]`) and adds the changes to `CHANGELOG.md`, from the [conventional commits](https://www.conventionalcommits.org) since the last release: `fix:` bumps the patch version, `feat:` the minor version (while we're at 0.x), and a breaking change (`feat!:`, or one `cargo-semver-checks` finds in the libraries) the minor version too. Edit the changelog in the PR if you like.
2. Merging the release PR publishes the three crates to crates.io (with [trusted publishing](https://crates.io/docs/trusted-publishing), so there's no token in the repository), tags `v<version>`, creates the GitHub release with the changelog entry, and attaches the archives listed in [docs/user/installation.md](docs/user/installation.md).

The release PR is opened with the workflow's `GITHUB_TOKEN`, so CI doesn't run on it: close and reopen it to run CI. A `RELEASE_PLZ_TOKEN` secret (a fine-grained personal access token with read and write access to contents and pull requests on this repository) makes CI run on it automatically.

To try the builds without releasing, run the Release workflow by hand (Actions > Release > Run workflow): the archives are uploaded as workflow artifacts.

The crates are published together, at one version: `rfluence-convert` and `rfluence-client` (libraries the command depends on) and `rfluence` (the command; `cargo install rfluence`). The packages leave out `tests/` and `examples/`, which need `fixtures/` from the repository. To check the packages locally:

```shell
cargo publish --workspace --dry-run    # packages each crate and builds it from the package alone
```

A new crate in the workspace has to be published by hand the first time (`cargo publish -p <crate>` with a crates.io token), because trusted publishing can't create crates; then add a trusted publisher for it on crates.io (repository `ckedwards/rfluence`, workflow `release.yaml`).

