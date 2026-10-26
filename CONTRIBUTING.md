# Contributing

Thank you for helping make file organization safer.

## Development workflow

1. Open an issue for significant behavior or architecture changes.
2. Create a focused branch from an up-to-date `main`:
   `feat/short-description`, `fix/short-description`,
   `docs/short-description`, or `chore/short-description`.
3. Keep commits cohesive and use concise imperative subjects.
4. Keep changes within the existing crate boundaries.
5. Add tests for success, collision, and failure paths.
6. Run formatting, Clippy with warnings denied, and all workspace tests.
7. Update user and safety documentation when behavior changes.
8. Open a pull request against `main` and merge only after CI passes.

Do not commit directly to `main` after the initial repository bootstrap. Keep
each pull request limited to one feature, fix, or maintenance concern. Describe
filesystem-safety implications explicitly.

Pull requests should explain the affected crates, configuration or migration
impact, validation commands, platform coverage, and data-integrity reasoning.
Never include personal paths, files, credentials, databases, or generated data.

By contributing, you agree that your contribution is licensed under either
Apache-2.0 or MIT, at the user's option.
