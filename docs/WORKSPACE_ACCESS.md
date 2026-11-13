# Workspace access and project file tools

## Access grants

The Workspace view requires the user to enter a project directory path and explicitly grant access. A grant is stored in process memory only and is discarded when Spindle exits. Revoke access from the same view at any time.

Read access is enabled by a grant. File writes are disabled by default and require the separate **Allow file writes** option. Regranting a path replaces the current workspace session.

## Available operations

| Operation | Limit / behavior |
|---|---|
| Directory listing | Direct children only; at most 500 entries per call; symbolic links and secret paths are omitted. |
| Read text file | UTF-8 only; 1 MiB maximum. |
| Write text file | Requires write grant; UTF-8 text only; 1 MiB maximum; writes use a temporary sibling file followed by rename. A SHA-256 revision captured at open is checked before save to reject edits made externally after the file was opened. |
| Search | Literal case-insensitive text search; 2,000 files, 16 MiB total scanned bytes, 512 KiB per file, 100 matches, and 16 directory levels maximum. |

Paths are relative to the canonicalized grant root. Absolute paths, parent traversal, link traversal, and special files are rejected. Directory search skips common generated folders such as `.git`, `node_modules`, `target`, `dist`, and `build`.

## Secret-file exclusions

Common environment and credential material is excluded from listing, reading, searching, and writing. Current name filters include `.env*`, `.npmrc`, `.pypirc`, SSH private key names, common cloud credential files/directories, and `.pem`, `.key`, `.p12`, `.pfx`, and `.asc` files.

Before saving, Spindle compares the currently stored file with the SHA-256 revision captured when the editor opened it. A mismatch rejects the save and asks the user to reopen the file. This reduces accidental lost updates but is not a complete atomic compare-and-swap and does not eliminate every filesystem race.

These name filters are a defensive default, not a complete secret scanner. Projects can store credentials under arbitrary names; users should not grant an untrusted agent access to a directory containing secrets.

## Security boundary and current limitations

The Tauri command boundary routes all workspace operations through a Rust `WorkspaceManager`; frontend checks are not treated as authorization. Each operation validates the session ID and the current read/write grant before resolving a relative path.

This is an application-level permission gate, not a Windows sandbox. Current operations use standard filesystem path resolution and metadata checks. They are not yet implemented with handle-relative APIs and do not claim full protection from every reparse-point or time-of-check/time-of-use race. Do not treat this layer as sufficient for autonomous execution of untrusted model output.

No shell, terminal, process-launch, network, Git write, delete, rename, or recursive bulk-edit command is exposed by this workspace slice. The user can manually edit a selected text file in the Workspace view when write permission was granted.

## Before autonomous tool use

Complete handle-relative and reparse-point-safe path operations, conflict detection and undo, OS-enforced isolation, a general permission broker, process and network policy, and Windows threat-driven integration tests before routing model-generated plans into these file tools.
