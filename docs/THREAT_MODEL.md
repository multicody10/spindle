# Threat Model

## Assets

- Source code and project files.
- API keys, environment variables, tokens, SSH material, and signing keys.
- Model files, model metadata, session state, and configuration profiles.
- Local inference endpoints and update artifacts.
- Integrity of the installer and future agent operations.

## Adversaries and untrusted input

- Malicious or compromised project repositories.
- Prompt injection embedded in source comments, documentation, terminal output, websites, MCP responses, or skills.
- Malformed GGUF metadata intended to exhaust memory or trigger parser bugs.
- Compromised model/provider endpoints.
- Tampered release or update artifacts.
- Malicious or compromised dependencies and CI actions.

## Current mitigations

- The UI has a narrow Tauri command surface; no shell or arbitrary filesystem APIs are exposed.
- Production CSP restricts scripts and network destinations.
- Provider discovery and chat are restricted to fixed loopback HTTP endpoints, disable proxies and redirects, and apply request, response, frame, and history limits.
- Chat requests validate message roles, sizes, model identifiers, context, and generation ranges in Rust; active requests can be cancelled.
- Model inspection is read-only and uses bounded GGUF metadata parsing.
- Database inputs are parameterized SQL statements.
- Settings are validated in Rust before being saved.
- Workspace access is granted only by an explicit user action and is held in memory for the current application session; it can be revoked. The shared permission broker applies operation-to-scope checks and persists bounded resource-tagged decisions in SQLite.
- Workspace reads and writes use relative paths under a canonicalized root, reject parent traversal and links, filter common secret files, and enforce per-operation size/count limits.
- File changes require a separate `canWrite` grant; default grants are read-only. Workspace edits are staged as diff previews and applied only after an explicit user action, with SHA-256 conflict checks and guarded undo. Proposal history remains bounded process memory; permission audit history is durable SQLite data with retention limits.
- Release workflows require updater signing material and refuse to proceed when it is absent.
- GitHub Actions use read-only permissions for validation, pinned action revisions, and a separate write-enabled tag release workflow.

## Known gaps

- A Windows AppContainer/LPAC worker now serves fixed user-triggered and agent-requested Cargo validation. It has no network capabilities, a sanitized environment, bounded output, and Job Object memory/CPU/time limits. It has not passed the full Windows adversarial integration suite and does not support arbitrary commands.
- Model-authored task file operations execute in the main application process and are constrained by the permission broker plus a detached Git worktree. They are not OS-isolated inside the AppContainer worker. The fixed Cargo validation process alone runs inside AppContainer/LPAC. Transcripts, task events, state transitions, and patch-application journals persist; startup recovery inspects in-flight effects rather than replaying model operations. Applying changes to the primary project remains a separate permission-checked action with guarded undo.
- Tool budgets and output are bounded. Skills and project memories are untrusted advisory context and cannot grant capabilities.
- The in-process broker provides session scopes and operation-aware auditing, but does not yet enforce resource-specific ACLs. The Cargo validation worker now applies and verifies a 64-process cap, per-process and aggregate memory limits, and no-breakaway policy before resume; adversarial Windows tests are still required. Explicit restricted-token and process-mitigation policies remain outstanding.
- The workspace file service is not yet handle-relative and has not been proven race-resistant against all Windows reparse-point/TOCTOU conditions.
- Spindle can send chat to already-running Ollama or LM Studio processes over loopback. Those provider processes remain externally managed.
- Spindle can install pinned llama.cpp Windows x64 CPU/Vulkan archives after SHA-256 verification, then launch one managed llama-server.exe with fixed arguments and a loopback-only bind address. It does not enable llama.cpp agent tools, MCP, or filesystem access.
- The autonomous agent loop can call typed workspace tools, but those operations execute in the main application process. Detached task worktrees and path-based permission checks reduce scope but are not an OS sandbox. The AppContainer worker runs only fixed Cargo validation; moving model-authored tool execution behind a restricted worker boundary is still required.
- NVIDIA telemetry does not prove that a matching CUDA or Vulkan inference backend is installed.
- Updater keys and Authenticode certificates must be provisioned by the repository owner before signed releases can ship.

## Required before autonomous execution

- A threat model review for each execution backend.
- OS-enforced workspace isolation and race-resistant path resolution before autonomous or untrusted tool execution.
- OS-enforced filesystem, process, and network restrictions.
- Safe environment variable allowlists and handle inheritance controls.
- Explicit user approval and revocation.
- Command identity and argument separation, not shell substring filtering.
- Process tree limits, wall-clock timeout, output caps, cancellation, and cleanup.
- Path operations resilient to junctions, symlinks, reparse points, and TOCTOU races.
- Prompt injection treated as untrusted data, never as permission.
- Security regression tests under a low-privilege Windows account.
