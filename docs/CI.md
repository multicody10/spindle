# CI and validation strategy

Spindle keeps routine pull request feedback fast and reserves packaging and security scans for deliberate runs.

## On every pull request update

The **Continuous integration** workflow runs two parallel Windows jobs:

- Frontend: npm ci, TypeScript type-check, and Vite production build.
- Rust: formatting, Clippy with warnings denied, and the locked Rust test suite.

The workflow does not run again for the merge push to main. The pull request is the validation boundary, so a successful PR run is not repeated unchanged immediately after merging.

## Full Windows package validation

The NSIS packaging job is intentionally excluded from routine pull request runs. For an installer-only test, run the **Continuous integration** workflow, not **Windows release**.

1. Open **Actions → Continuous integration**.
2. Select **Run workflow**.
3. Choose the branch containing the pull request changes.
4. Start the workflow.

A manual run executes the frontend and Rust jobs plus the unsigned Windows NSIS build and installer verification. The artifact is retained for seven days.

## Signed Windows releases

Use **Actions → Windows release → Run workflow** and enter the next version tag, such as v0.1.1. Dispatch the workflow from main.

The release workflow automatically:

1. Validates the requested semantic version, current version consistency, updater configuration, and signing secret before compiling.
2. Updates package.json, package-lock.json, src-tauri/tauri.conf.json, src-tauri/Cargo.toml, and the local spindle entry in src-tauri/Cargo.lock.
3. Commits the version bump directly to main.
4. Builds, signs, and publishes the Windows NSIS installer and updater manifest from that exact commit.
5. Uploads the SHA-256 checksum.

The workflow refuses older versions, mismatched version declarations, existing tags for a new version, an unconfigured updater key, missing signing secrets, or manual dispatches from another branch. The release tag is created from the version-bump commit by the Tauri release action.

If the build fails after the version commit, rerun **Windows release** with the same tag. The workflow recognizes that the version is already committed and resumes without creating a second version commit. If a tag already exists, it reuses the tagged commit so the build source remains stable.

A normal version bump is therefore part of the release run. Do not manually edit version files or create the tag first. The signing private key remains in the GitHub Actions secret and is never written to the repository.

## Security scans

- **CodeQL** runs every Tuesday and can also be started manually.
- **Rust supply-chain checks** runs every Wednesday and can also be started manually. It checks Rust advisories, licenses, banned crates, and source policies.

These scans are deliberately not duplicated for every pull request commit and merge push. Run either manually when changing the corresponding security-sensitive areas.

## Release validation

The Windows release workflow performs frontend type-checking, Rust formatting, Clippy, Rust tests, and signed Windows packaging and publishing. These steps run only after the version and signing preflight succeeds.
