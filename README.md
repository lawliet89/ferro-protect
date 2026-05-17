# ferro-protect

Async Rust client and CLI for the [UniFi Protect](https://ui.com/) local integration
API. Targets Protect application version **6.2.83**. The workspace publishes two
crates: the `ferro-protect` library and the `ferro-protect-cli` binary
(`ferro-protect`).

## Status

Pre-0.1.0. Built phase by phase against the OpenAPI spec hosted at
<https://github.com/beezly/unifi-apis>.

## Clone

```sh
git clone --recurse-submodules https://github.com/lawliet89/ferro-protect.git
```

If you forgot `--recurse-submodules`:

```sh
git submodule update --init --recursive
```

## Build

```sh
cargo build --workspace
```

## Local checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo deny check
```

Install the pre-commit hook to run fmt and clippy automatically:

```sh
ln -s ../../scripts/pre-commit .git/hooks/pre-commit
chmod +x scripts/pre-commit
```
