# Install, upgrade, uninstall

Annotary is one binary, for macOS and Linux on arm64 and x86_64. The binary is the same whichever way
you install it. To run it as a service on a Linux server, see
[`ops-health-check.md`](ops-health-check.md); everything below is for your own machine.

## Install

**Homebrew** (macOS or Linux):

```sh
brew install josh-andrews-dev/tap/annotary
```

**The install script** (macOS or Linux, no Rust or Homebrew needed). It downloads the binary for your
machine, checks its checksum and puts it in `~/.local/bin`:

```sh
curl -fsSL https://github.com/josh-andrews-dev/annotary/releases/latest/download/annotary-installer.sh | sh
```

**A tarball**, from [the releases page](https://github.com/josh-andrews-dev/annotary/releases). Each is
`annotary-<target>.tar.gz`, where `<target>` is `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`aarch64-unknown-linux-gnu` or `x86_64-unknown-linux-gnu`, with its checksum beside it as
`annotary-<target>.tar.gz.sha256` and all of them in `sha256.sum`. Check it, unpack it and put the binary on
your `PATH`:

```sh
target=x86_64-unknown-linux-gnu
base=https://github.com/josh-andrews-dev/annotary/releases/latest/download
curl -fsSLO "$base/annotary-$target.tar.gz"
curl -fsSLO "$base/annotary-$target.tar.gz.sha256"
sha256sum -c "annotary-$target.tar.gz.sha256"       # on a Mac: shasum -a 256 -c
tar -xzf "annotary-$target.tar.gz"
install -m 755 "annotary-$target/annotary" ~/.local/bin/annotary
```

**From source**, with Rust stable:

```sh
cargo install --locked --git https://github.com/josh-andrews-dev/annotary annotary
```

A build from source reports version `0.0.0` and gets no update notice.

Then `annotary --version`, and the [Quickstart](../README.md#quickstart).

## Upgrade

- Homebrew: `brew upgrade josh-andrews-dev/tap/annotary`
- The install script: run the same line again.
- A tarball: repeat the steps with the new one.
- From source: run the same `cargo install` line with `--force`.

Then restart the server if you run one (`annotary daemon restart`, or stop and start `annotary serve`): it
keeps running the old binary until then. A new binary migrates your store the first time it opens it, and
the previous binary then refuses that store, so take an `annotary export FILE.tar` first if you may want to
go back.

### The update notice

`annotary --version` on a terminal, and `annotary serve` when it starts, say when a newer release exists.
They ask GitHub's Releases API (`api.github.com`) at most once a day and keep the answer in
`update-check.json` in your data dir. The request sends nothing but your version, as the `User-Agent`. Other
commands never ask and never print it, and it stays quiet if you are offline. To turn it off, add this to the
top of `~/.annotary/config.toml`:

```toml
update_check = false
```

or set `ANNOTARY_UPDATE_CHECK=off` in the environment.

## Uninstall

Remove the binary:

- Homebrew: `brew uninstall josh-andrews-dev/tap/annotary`
- The install script: `rm ~/.local/bin/annotary`, and the `annotary` folder it made in `~/.config`.
- A tarball: remove the binary you installed.
- From source: `cargo uninstall annotary`

If you ran it as a LaunchAgent, run `annotary daemon uninstall` first. Your Documents are not removed with
the binary: they are in `~/.annotary`, or wherever `--data-dir` or `ANNOTARY_HOME` pointed. Delete that folder
when you are sure you are done with them; nothing else keeps a copy.
