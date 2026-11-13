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
- Release workflows require updater signing material and refuse to proceed when it is absent.
- GitHub Actions use read-only permissions for validation, pinned action revisions, and a separate write-enabled tag release workflow.

## Known gaps

- No AppContainer/LPAC execution worker exists.
- No permission broker, network isolation, restricted token, or process Job Object policy exists.
- No filesystem handle-relative/reparse-point-safe workspace mutation service exists.
- Spindle can send chat to an already-running Ollama or LM Studio process over loopback, but does not install, launch, stop, or configure provider processes.
- NVIDIA telemetry does not prove that a matching CUDA or Vulkan inference backend is installed.
- Updater keys and Authenticode certificates must be provisioned by the repository owner before signed releases can ship.

## Required before autonomous execution

- A threat model review for each execution backend.
- OS-enforced filesystem, process, and network restrictions.
- Safe environment variable allowlists and handle inheritance controls.
- Explicit user approval and revocation.
- Command identity and argument separation, not shell substring filtering.
- Process tree limits, wall-clock timeout, output caps, cancellation, and cleanup.
- Path operations resilient to junctions, symlinks, reparse points, and TOCTOU races.
- Prompt injection treated as untrusted data, never as permission.
- Security regression tests under a low-privilege Windows account.
