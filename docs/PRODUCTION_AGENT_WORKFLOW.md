# Production Agent Workflow

Status: the approved project-task workflow uses provider-native tool calling with a fixed typed tool set. Model calls are parsed from Ollama tool-call frames or OpenAI-compatible streamed `tool_calls` deltas, then routed through the task tool dispatcher and session permission broker. Tools execute against a detached task worktree; final changes require separate user review and apply. The tool host and path-based file operations currently run in the main Spindle process and are not OS-isolated. This does not enable arbitrary process execution, external network, MCP, or parallel agents.

## Runtime architecture

Spindle separates orchestration from the execution workspace:

- **Control plane:** Tauri/Rust owns project identity, provider selection, permission checks, task state, SQLite persistence, event ordering, cancellation, and final application.
- **Model adapter:** sends the fixed function schemas with each agent request. Ollama uses its native `tools`, `message.tool_calls`, and `tool_name` conventions. LM Studio and Spindle llama.cpp use OpenAI-compatible `tools`, streamed `delta.tool_calls`, assistant tool-call messages, and call-ID-linked tool results. Models/providers may vary in tool-call support; unsupported responses fail as provider errors rather than silently falling back to pretend execution.
- **Tool broker:** parses typed arguments, checks the session permission broker for every operation, applies path and size constraints, and returns structured results.
- **Task workspace:** every task writes to its own detached Git worktree. The registered project working tree is not edited by the model.
- **Validation worker:** fixed commands execute in the Windows AppContainer/LPAC worker with no network capabilities, a Job Object, bounded output, and offline dependencies.
- **Review/application:** completion requires a final diff read after the last file mutation. Applying the patch to the registered project is a separate user action.

## Task state machine

    planning -> awaiting_approval -> approved -> running -> completed
                                      |             |-> failed
                                      |             |-> cancelled
                                      |             |-> interrupted -> running
                                      |             |-> failed -> running
                                      |             |-> cancelled -> running
                                      completed -> applied -> undone

The durable transcript is ordered by task and sequence. Native assistant tool-call batches and each result are stored as separate transcript entries, while lifecycle events are persisted independently. On startup, running tasks become interrupted. If a tool-call batch exists without a committed result, recovery inserts an explicit outcome-unknown tool result and never automatically replays that call. The model is instructed to inspect current files and Git state before deciding what to do next.

Application uses a SQLite journal with the patch and SHA-256 digest. Startup recovery distinguishes a patch that was applied, not applied, or conflicted. Undo is guarded by a reverse-patch check and refuses to overwrite intervening user edits. This is recoverable, journaled application, not an atomic multi-file filesystem transaction under every possible power-loss or storage failure.

## Model protocol and tool surface

The model returns provider-native function tool calls. OpenAI-compatible streamed call fragments are accumulated by call index; identifiers and function names are validated, argument JSON must decode to an object, and oversized or malformed calls fail closed. Ollama tool arguments are parsed from the provider's structured JSON object payload. Multiple calls per turn are supported.

The loop has a 30-minute total runtime deadline, a 48-turn ceiling, a 64-call total budget, an 8-call per-turn limit, a 64 KiB aggregate argument budget per turn, bounded tool outputs, and duplicate-read suppression. Context selection preserves system instructions, the task goal, and recent complete tool-call/result groups. Cancellation signals the active model request and any cancellable validation process.

Current tools:

| Tool | Purpose |
|---|---|
| list_files | Bounded workspace directory listing |
| read_file | UTF-8 read with SHA-256 revision and line bounds |
| search_files | Bounded literal text search |
| list_skills / load_skill | Discover and read project/global Agent Skills |
| create_directory / create_file | Create new paths without overwrite |
| write_file | Revision-checked full-file replacement |
| edit_file | Unique-anchor localized replacement |
| delete_file | Revision-checked regular-file deletion |
| git_status | Inspect detached-worktree status |
| git_history | Read bounded recent commit history |
| get_diff | Review tracked and untracked task changes |
| validate | Run one of the fixed Cargo validation commands |

There is deliberately no arbitrary shell, executable, package installation, external network, MCP, or parallel-agent tool. Repository instructions, source, docs, model output, Skills, and project memories are untrusted data; none can expand the tool surface or permissions.

## Permission model

The runtime checks the existing broker for each operation:

- ProjectRead for inspection and review.
- ProjectWrite for proposed edits and their application inside the isolated task worktree.
- ProcessExecution for restricted validation.

Moving a completed patch into the registered project requires a separate user action and a separate FileChangeApply authorization. Undo is a separate FileChangeUndo authorization. The current broker scopes map those operation types to ProjectWrite but retain distinct resource-tagged audit events. No project memory, Skill, task plan, or model response can grant permission.

## Workspace and patch limits

- Relative paths only; no traversal, absolute paths, or Git metadata.
- Secret-oriented paths and common credential files are excluded.
- UTF-8 text files only for model file tools; 1 MiB maximum per file.
- Directory listing, search, read ranges, tool output, total conversation context, number of messages, model turns, and tool calls are bounded.
- Task diffs are capped at 2 MiB and 500 changed paths for review/application.
- Changed paths are checked for links/reparse points, special files, size, UTF-8, and containment before patch application.
- Git hooks and external credential helpers remain disabled for the controlled Git operations.

## Validation profile

The current restricted worker exposes **Cargo format/check/test only**, and only on supported Windows installations. It uses AppContainer + LPAC, no network capabilities, an allowlisted environment, read-only toolchain access, a private offline Cargo cache, a 2 GiB memory cap, an 80% CPU cap, a ten-minute timeout, bounded output, and kill-on-job-close. Cancellation terminates the worker Job Object.

Validation is not general command execution. The worker does not yet provide Node/Vite/Python/Go/.NET test profiles. A task can edit those projects, but it cannot truthfully claim that their ecosystem-specific checks passed through Spindle. This is a known product gap, not an invitation to use an unsandboxed terminal fallback.

## Explicitly unproven boundaries

1. Windows AppContainer/LPAC access-denial behavior still needs adversarial integration tests on supported Windows versions.
2. The filesystem service is path-based. It rejects common symlink/reparse traversal, but it is not yet a handle-relative, race-proof Windows filesystem broker.
3. Job Object process-count and breakaway restrictions are not implemented or verified.
4. The Git patch journal provides restart classification and guarded reverse application, not crash-atomic transactions across every affected file.
5. Model tool calling is a strict JSON text protocol, not provider-native structured tool calling or token-level streaming tool events.
6. No general egress broker, MCP runtime, arbitrary shell, or parallel agents are enabled.
7. Context is bounded by preserving the goal and recent evidence; a full semantic summarization/compaction system remains future work.

Do not label the worker a hardened sandbox until the Windows security test matrix passes and the remaining filesystem/process boundary gaps are closed.
