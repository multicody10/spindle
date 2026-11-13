# Contributing

Thanks for helping improve Spindle. The repository is public and all contributions should preserve the project's local-first, model-agnostic, and security-first goals.

## Before opening a pull request

- Explain the problem and intended behavior.
- Keep changes focused and reviewable.
- Add or update tests for parser, optimizer, persistence, and validation changes.
- Do not add credentials, private configuration, model weights, local databases, or user project data.
- Do not make model-generated content authoritative for permission decisions.
- Do not add command execution without a reviewed OS isolation design.

## Local checks

Run npm install and npm run check, then run cargo fmt, cargo clippy, and cargo test against src-tauri/Cargo.toml.

## Design expectations

Use Rust for privileged logic and typed IPC. Expose the minimum required Tauri commands. Make unsupported provider settings explicit. Keep model loading parameters separate from token-generation parameters. Do not claim a recommendation was applied unless a runtime adapter verifies the effective runtime state.

## Licensing

By contributing, you agree that your contributions are provided under the repository's MIT License.
