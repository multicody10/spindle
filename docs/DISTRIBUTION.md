# Windows Distribution and Updates

## Installation scope

The Windows NSIS installer is configured for `perMachine`. A fresh installation defaults to the 64-bit Program Files directory and requests administrator approval. The updater's Windows mode remains `passive`, which displays installation progress and allows the NSIS installer to request elevation. Do not change updater mode to `quiet` for machine-wide installs.

CI and the release preflight validate both settings:
- `bundle.windows.nsis.installMode = perMachine`
- `plugins.updater.windows.installMode = passive`

## One-time transition from the first release

Spindle `v0.1.0` was initially published with a per-user installer. That copy installs under `%LOCALAPPDATA%`. A later per-machine installer defaults to Program Files and reads the machine-wide install registry context; it does not automatically migrate a prior per-user installation recorded under the current user's registry context.

Before moving an existing `v0.1.0` installation to Program Files:

1. Use Windows **Installed apps** to uninstall the existing per-user Spindle copy.
2. Leave the uninstaller's application-data deletion option unchecked. This preserves Spindle's local configuration database and saved presets.
3. Install the next per-machine NSIS release and approve the UAC prompt.
4. Confirm the executable is under `C:\Program Files\Spindle\`.

Do this one time when moving from the first per-user release. Do not rely on the in-app updater to migrate installation scope. Once Spindle is installed per-machine, subsequent signed updater releases use the same per-machine NSIS configuration and default to Program Files.

## Verification still required

The CI package job validates the installer artifact and enforces the configured install/update modes. A manual Windows test is still required to verify the first per-user to per-machine transition, preserve user data, and then verify an in-app update from the Program Files installation.
