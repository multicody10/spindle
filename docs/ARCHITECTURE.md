# Architecture

## Foundation layout

Spindle uses a native Rust/Tauri 2 core and a React/TypeScript/Vite interface. Rust owns hardware inspection, GGUF parsing, optimization calculations, configuration validation, and SQLite persistence. The frontend is a presentation layer that calls a narrow typed command surface through Tauri IPC.

Current request path:

React / TypeScript → typed IPC bridge → Tauri command boundary → hardware inventory, bounded GGUF parser, explainable optimizer, fixed-loopback provider discovery and streaming chat adapters, verified llama.cpp runtime installer/process manager with a shared inference scheduler, temporary workspace permission manager and bounded project file service, and SQLite persistence.

## Trust boundaries

- The frontend is not a security authority.
- The model file is treated as untrusted binary input. The GGUF parser bounds metadata counts, string lengths, and array lengths.
- The model configuration is validated again in Rust before persistence.
- Hardware inventory reads the human-readable processor model from the Windows processor registry, checks NVIDIA through nvidia-smi and NVML, and uses the Windows display-adapter registry as a detection fallback when live VRAM telemetry is unavailable.
- SQLite lives in the operating system application data directory, not in the project folder.
- Ollama and LM Studio discovery/chat use fixed IPv4 loopback endpoints and never launch or control those external provider processes.
- The managed llama.cpp installer uses fixed official release URLs, pinned SHA-256 digests, bounded downloads, safe ZIP path checks, and per-user application-data storage.
- The process manager launches only the installed, verified llama-server.exe with typed settings and fixed 127.0.0.1 binding; it does not invoke a shell or accept arbitrary executable paths.
- The shared inference manager admits at most one active generation per application session and bounds total concurrency to the configured llama.cpp parallel slots. Each session retains an assigned server slot while it remains in the in-memory LRU set; chat requests pin `id_slot` and enable `cache_prompt` for KV-prefix reuse. Slot eviction erases the old server cache before reuse. Excess requests wait in a bounded, cancellable queue; stopping or replacing the model is rejected while requests are active or queued.
- Chat streams provider tokens back to the UI through scoped Tauri events; each request can be cancelled independently.
- Chat history exists only in the active window and is not persisted.
- Workspace access is granted explicitly for one project root and one temporary application session. Its session is registered with the shared permission broker: ProjectRead is the baseline, and ProjectWrite is added only when the user enables file writes. Broker decisions are operation-aware and resource-tagged, persisted in SQLite with a 10,000-event retention cap.
- Workspace file tools accept relative paths only, reject parent traversal and links, exclude common secret-file names, limit text reads and proposed writes to 1 MiB, cap directory listings at 500 entries, and bound recursive text search. File changes are staged as bounded diff previews, then explicitly applied or discarded; an applied change can be undone only while its post-apply revision remains unchanged. Proposal history and original text are bounded process memory, not durable checkpoints.
- Workspace grants are an application-level IPC gate, not an OS sandbox. Operations are not yet handle-relative and do not claim full race-resistant protection against every Windows reparse-point/TOCTOU attack.
- No shell execution, autonomous file edits, MCP tool hosting, or agent loop is exposed in this milestone. The llama.cpp server is not started with its agent, tools, MCP, or filesystem features.

## Planned architecture

The next architecture layers are:

1. Provider capability registry and managed runtime adapters, extending the existing Ollama and LM Studio streaming chat boundary.
2. [x] Central inference runtime manager for one shared loaded weight set, per-session request leases, a bounded queue, and configured parallel-slot admission. Per-session KV-prefix reuse is implemented in server memory; disk-backed KV restoration across runtime restarts remains future work.
3. Session coordinator and project-scoped persistence.
4. [x] General scope permission broker with operation-aware authorization and bounded audit events; path/resource restrictions remain enforced by each owning service.
5. Restricted execution worker with Windows AppContainer/LPAC where compatible, plus resource and process-tree controls.
6. [x] Manual workspace stage/apply/discard and revision-guarded undo flow. [ ] Handle-relative/reparse-aware file service, durable checkpoints, and race resistance.
7. Agent orchestration and verifiable tool-result state machine.
8. Secure updater and release verification.

## Model configuration precedence

The target precedence is session override, model-specific override, automatic recommendation, and provider/runtime default. The current database stores per-model, per-profile configurations and named presets. Session overrides and provider defaults are not yet implemented.

## Inference runtime

llama.cpp is the initial managed local runtime target, not a custom inference engine. Spindle loads one model process and shares that loaded weight set across concurrent sessions. The manager fingerprints the model path, runtime build, backend, and load-time settings, then admits requests through a semaphore sized to the configured `--parallel` slots. A session may have only one generation in flight; excess sessions wait in a bounded queue and can cancel while waiting. Spindle maps each retained session to one explicit llama.cpp slot and sets `cache_prompt: true`, allowing llama.cpp to reuse the common prompt prefix and its KV state across turns. When the bounded session-to-slot map is full, the least recently used inactive session is evicted and its server slot is erased before reassignment. Slot state is RAM-only and is lost when the managed server exits; it is not serialized across application restarts. Each request still sends its conversation history so the server can identify the reusable prefix. Future adapters must distinguish loaded model weights, runtime configuration, session context, KV cache, and active request scheduling.


## Supervised agent execution

Approved task plans do not themselves authorize file writes or process execution. The user must separately grant the session's project-write scope before Spindle creates a detached Git worktree. Worktree creation requires a clean registered repository and records its base commit in the project database. Removal is refused for dirty worktrees or when the detached HEAD has advanced beyond that recorded base. Git status/history are read-only; task diffs are bounded and currently show tracked-file changes only, with untracked files reported separately. Git operations disable repository hooks and refuse repositories with configured external clean/smudge/process filters, which could otherwise execute code in the Spindle process.

Validation is a separate, explicit user action and requires the process-execution scope. The current Windows worker uses an AppContainer with LPAC, grants access only to the task worktree, its private profile, the Rust toolchain, and an isolated offline Cargo cache, and launches only fixed `cargo fmt --check`, `cargo check --locked`, or `cargo test --locked` operations. It grants no network capabilities, uses a Job Object with memory/CPU limits and kill-on-close, and returns bounded, credential-scrubbed output. It never falls back to an unsandboxed process. Non-Windows platforms report the worker as unavailable.

The worker is a constrained validation runner, not yet a general coding-agent execution host. Model-authored file operations in task worktrees, transactional diff application back to the primary working tree, handle-relative/reparse-point-safe file access, explicit process mitigations, and Windows adversarial integration tests remain gated. Workspace proposal state is still in memory; durable transaction recovery is not claimed.
