# CI and validation strategy

Spindle keeps routine pull request feedback fast and reserves packaging and security scans for deliberate runs.

## On every pull request update

The **Continuous integration** workflow runs two parallel Windows jobs:

- Frontend: `npm ci`, TypeScript type-check, and Vite production build.
- Rust: formatting, Clippy with warnings denied, and the locked Rust test suite.

The workflow does not run again for the merge push to `main`. The pull request is the validation boundary, so a successful PR run is not repeated unchanged immediately after merging.

## Full Windows package validation

The NSIS packaging job is intentionally excluded from routine pull request runs. Run the full suite when a substantial change is ready for final validation:

1. Open **Actions → Continuous integration**.
2. Select **Run workflow**.
3. Choose the branch containing the pull request changes.
4. Start the workflow.

A manual run executes the frontend and Rust jobs plus the unsigned Windows NSIS build and installer verification. The artifact is retained for seven days.

The actual signed installer is still built and verified by the Windows release workflow. Packaging validation does not use or require the updater private key.

## Security scans

- **CodeQL** runs every Tuesday and can also be started manually.
- **Rust supply-chain checks** runs every Wednesday and can also be started manually. It checks Rust advisories, licenses, banned crates, and source policies.

These scans are deliberately not duplicated for every pull request commit and merge push. Run either manually when changing the corresponding security-sensitive areas.

## Release validation

The Windows release workflow remains unchanged. A release run performs frontend type-checking, Rust formatting, Clippy, Rust tests, and the signed Windows packaging and publishing steps.

This retains full validation at the release boundary while avoiding a second NSIS build on ordinary merge pushes.
