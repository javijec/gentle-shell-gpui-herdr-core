## What and why

<!-- What changes for the user, and why. Link the issue if there is one. -->

## How it was tested

<!-- Commands, platforms, a temporary-HOME smoke test (docs/development.md). -->

## Security

Mark what this PR does; a maintainer reads those parts line by line (docs/security.md).

- [ ] Adds or bumps dependencies (`Cargo.toml` / `Cargo.lock`): which and why:
- [ ] Starts processes, opens network connections or downloads files
- [ ] Reads or writes files outside the app's data dir (configs, credentials, home)
- [ ] Touches CI (`.github/`), `build.rs`, `packaging/` or the self-updater
- [ ] Adds binary files (images, fonts, archives): where they come from:
- [ ] None of the above

## Checklist

- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`
- [ ] One change per PR; refactors separate and marked `(no behavior change)`
