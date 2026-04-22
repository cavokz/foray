# Contributing

## Development Setup

After cloning, enable the pre-commit hooks (formatting + lint checks):

```sh
git config core.hooksPath .githooks
```

This runs `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings` before every commit.
