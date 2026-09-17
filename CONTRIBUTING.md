# Contributing

## Reporting bugs

Please include the build hash or commit, the operating system, exact steps to reproduce, and the expected versus actual behavior. If the issue involves runtime behavior, attach the relevant log excerpt captured with `RUST_LOG=debug`.

## Pull requests

Target `main` unless a maintainer asks otherwise. Before opening a PR, run `cargo test --workspace` from the repository root and `npx tsc --noEmit` from `ui/`; both must pass. Keep each PR to one logical change.

## Commit style

Use Conventional Commits: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`, or `test:`.
