# Installation

## Prebuilt binaries

Binaries for Linux, macOS and Windows will be published on the [releases page](https://github.com/ckedwards/rfluence/releases). They aren't available yet; build from source for now.

## From source

You need Rust 1.85 or later ([rustup](https://rustup.rs) installs it). No other libraries are needed: HTTPS and the keyring support are built in.

```shell
git clone https://github.com/ckedwards/rfluence.git
cd rfluence
cargo install --path crates/rfluence-cli
```

This puts `rfluence` in `~/.cargo/bin`. If your shell can't find it, add that directory to your `PATH`, e.g. in `~/.bashrc` or `~/.zshrc`:

```shell
export PATH="$HOME/.cargo/bin:$PATH"
```

To update, pull and run the `cargo install` command again.

## Check it works

```shell
rfluence --version
rfluence --help
```

Next: [log in](authentication.md).
