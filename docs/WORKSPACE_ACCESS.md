# Workspace access and project file tools

## Access grants

The Workspace view requires the user to enter a project directory path and explicitly grant access. A grant is stored in process memory only and is discarded when Spindle exits. Revoke access from the same view at any time.

Read access is enabled by a grant. File writes are disabled by default and require the separate **Allow file writes** option. Regranting a path replaces the current workspace session.

## Available operations

| Operation | Limit / behavior |
|---|---|
| Directory listing | Direct children only; at most 500 entries per call; symbolic links and secret paths are omitted. |
| Read text file | UTF-8 only; 1 MiB maximum. |
| Stage a file change | Requires the ProjectWrite scope; UTF-8 text only; 1 MiB maximum. Captures the original text and SHA-256 revision, stores a bounded diff preview, and does not modify the file. |
| Apply reviewed change | Requires ProjectWrite and an explicit UI action. Rechecks the original SHA-256 revision immediately before atomically replacing the file through a temporary sibling. |
| Undo applied change | Requires ProjectWrite. Restores captured original text only when the file still matches the revision Spindle applied, preventing undo from overwriting later edits. |
| Search | Literal case-insensitive text search; 2,000 files, 16 MiB total scanned bytes, 512 KiB per file, 100 matches, and 16 directory levels maximum. |

Paths are relative to the canonicalized grant root. Absolute paths, parent traversal, link traversal, and special files are rejected. Directory search skips common generated folders such as `.git`, `node_modules`, `target`, `dist`, and `build`.

## Secret-file exclusions

Common environment and credential material is excluded from listing, reading, searching, and writing. Current name filters include `.env*`, `.npmrc`, `.pypirc`, SSH private key names, common cloud credential files/directories, and `.pem`, `.key`, `.p12`, `.pfx`, and `.asc` files.

Before saving, Spindle compares the currently stored file with the SHA-256 revision captured when the editor opened it. A mismatch rejects the save and asks the user to reopen the file. This reduces accidental lost updates but is not a complete atomic compare-and-swap and does not eliminate every filesystem race.

These name filters are a defensive default, not a complete secret scanner. Projects can store credentials under arbitrary names; users should not grant an untrusted agent access to a directory containing secrets.

## Security boundary and current limitations

The Tauri command boundary routes all workspace operations through both the shared Rust `PermissionBroker` and `WorkspaceManager`; frontend checks are not treated as authorization. The broker authorizes each operation against a project/session scope and records a bounded in-memory audit event with the requested resource identifier. The workspace manager independently validates its temporary session ID, read/write grant, and relative path before resolving it. Workspace grants register in the same broker, start with ProjectRead, and add ProjectWrite only when the user explicitly selects Allow file writes.

This is an application-level permission gate, not a Windows sandbox. Current operations use standard filesystem path resolution and metadata checks. They are not yet implemented with handle-relative APIs and do not claim full protection from every reparse-point or time-of-check/time-of-use race. Do not treat this layer as sufficient for autonomous execution of untrusted model output.

No shell, terminal, process-launch, network, Git write, delete, rename, or recursive bulk-edit command is exposed by this workspace slice. The user can stage a selected text file edit, review its diff, explicitly apply or discard it, and undo an applied change while its post-apply revision remains unchanged.

## Before autonomous tool use

The general scope broker and reviewed stage/apply/undo flow now exist for the manual Workspace editor. They are not sufficient for autonomous tools. Complete handle-relative and reparse-point-safe path operations, durable transaction/checkpoint recovery, isolated Git worktrees, the Windows AppContainer/LPAC worker, restricted token and Job Object policies, default-deny network controls, environment/secret redaction, and Windows threat-driven integration tests before routing model-generated plans into any mutating tool.
