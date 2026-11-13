# CI and validation strategy

Spindle keeps routine pull request feedback fast and reserves packaging and security scans for deliberate runs.

## On every pull request update

The **Continuous integration** workflow runs two parallel Windows jobs:

- Frontend: `npm ci`, TypeScript type-check, and Vite production build.
- Rust: formatting, Clippy with warnings denied, and the locked Rust test suite.

The workflow does not run again for the merge push to `main`. The pull request is the validation boundary, so a successful PR run is not repeated unchanged immediately after merging.

## Full Windows package validation

The NSIS packaging job is intentionally excluded from routine pull request runs. For an installer-only test, run the **Continuous integration** workflow, not **Windows release**. Run the full suite when a substantial change is ready for final validation:

1. Open **Actions → Continuous integration**.
2. Select **Run workflow**.
3. Choose the branch containing the pull request changes.
4. Start the workflow.

A manual run executes the frontend and Rust jobs plus the unsigned Windows NSIS build and installer verification. The artifact is retained for seven days.

The actual signed installer is built and published by **Windows release**. That workflow creates a public GitHub Release and requires the requested tag to match the version in `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`. It also requires the updater signing secrets. Do not use it as a package-only test. Packaging validation does not use or require the updater private key.

## Security scans

- **CodeQL** runs every Tuesday and can also be started manually.
- **Rust supply-chain checks** runs every Wednesday and can also be started manually. It checks Rust advisories, licenses, banned crates, and source policies.

These scans are deliberately not duplicated for every pull request commit and merge push. Run either manually when changing the corresponding security-sensitive areas.

## Release validation

The Windows release workflow performs frontend type-checking, Rust formatting, Clippy, Rust tests, and signed Windows packaging and publishing. Release validation steps run only after the initial version and signing preflight succeeds, so an invalid tag does not trigger additional compilation and test work.

This retains full validation at the release boundary while avoiding a second NSIS build on ordinary merge pushes.
