# Installation

## Prebuilt binaries

Releases after 0.1.0 have prebuilt binaries on the [releases page](https://github.com/ckedwards/rfluence/releases) (0.1.0 is only on crates.io: install it with Cargo). Each has an archive per platform, with a `.sha256` checksum:

| Platform | Archive |
| --- | --- |
| Linux, x86_64 | `rfluence-<version>-x86_64-unknown-linux-musl.tar.gz` |
| Linux, ARM64 | `rfluence-<version>-aarch64-unknown-linux-musl.tar.gz` |
| macOS, Apple Silicon | `rfluence-<version>-aarch64-apple-darwin.tar.gz` |
| macOS, Intel | `rfluence-<version>-x86_64-apple-darwin.tar.gz` |
| Windows | `rfluence-<version>-x86_64-pc-windows-msvc.zip` |

The Linux builds are static, so they run on any distribution. On Linux or macOS:

```shell
tar -xzf rfluence-<version>-<platform>.tar.gz
sudo mv rfluence-<version>-<platform>/rfluence /usr/local/bin/   # or any directory on your PATH
```

On Windows, unzip it and put `rfluence.exe` in a folder on your `PATH`. macOS may refuse to open a downloaded binary at first; allow it in System Settings > Privacy & Security, or run `xattr -d com.apple.quarantine /usr/local/bin/rfluence`.

## With Cargo

You need Rust 1.88 or later ([rustup](https://rustup.rs) installs it). No other libraries are needed: HTTPS and the keyring support are built in.

```shell
cargo install rfluence      # the latest release, from crates.io
```

Or the latest source:

```shell
cargo install --git https://github.com/ckedwards/rfluence rfluence
```

This puts `rfluence` in `~/.cargo/bin`. If your shell can't find it, add that directory to your `PATH`, e.g. in `~/.bashrc` or `~/.zshrc`:

```shell
export PATH="$HOME/.cargo/bin:$PATH"
```

To update, run the `cargo install` command again.

## Check it works

```shell
rfluence --version
rfluence --help
```

Next: [log in](authentication.md).
