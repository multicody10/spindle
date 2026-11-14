# Secure agent execution design

Status: native provider tool calling is enabled for approved project tasks. Ollama uses its native chat tool schema and tool-result message format; LM Studio and Spindle-managed llama.cpp use the OpenAI-compatible streamed `tool_calls` format. The agent loop persists assistant tool-call batches and individual tool results, enforces fixed tool and size budgets, and records task activity. File and Git tools operate only within a detached task worktree through the application permission broker. Moving the resulting patch into the registered project remains a separate, explicitly approved action. The tool host runs in the main Spindle process, and the file broker remains path-based rather than handle-relative/race-resistant. The fixed Cargo validation worker starts suspended, is assigned to a Job Object, applies and reads back active-process, per-process memory, aggregate job-memory, CPU, kill-on-close, and no-breakaway policy, and only then resumes its primary thread. This validation worker still requires Windows adversarial runtime acceptance before its containment claims are considered verified.

**Important execution boundary:** native tool calls and typed workspace operations execute inside the main Spindle application process. They are restricted by the fixed tool registry, the permission broker, and the detached task worktree, but are not executed inside the AppContainer worker. The workspace file service rejects common traversal/link cases but is not handle-relative and does not eliminate every Windows reparse-point/TOCTOU race. Only fixed Cargo validation commands run in AppContainer/LPAC today. Do not describe native tools as OS-isolated; the remaining filesystem race risk is explicit.

## Trust boundary

Spindle's UI and Rust application process are the agent orchestrator, or agent head. They retain project identity, session history, plans, permissions, provider configuration, and task events.

Model-generated code and commands must run in a separate worker process with a distinct security boundary. The worker is the agent's hands. It must not inherit the authority of the application process.

A permission broker alone is not a sandbox. Prompt instructions, path filtering, canonicalization, command allowlists, and process timeouts are useful defense in depth, but cannot contain arbitrary hostile code.

## Required Windows worker properties

Before enabling arbitrary process execution, untrusted package scripts, external network, MCP, or parallel agents, implement and validate all of the following:

1. **AppContainer or LPAC isolation.** Implemented for the fixed Cargo validation worker using a persistent AppContainer identity with LPAC and no network capabilities. ACL grants are limited to the task worktree, private worker profile/offline Cargo cache, and read-only Rust toolchain directories. Actual access-denied boundaries still require adversarial Windows integration tests.
2. **Restricted process creation.** The Cargo validation worker is now created suspended. The launcher attaches it to its Job Object before returning; Spindle then applies and queries the complete Job Object policy before resuming the one enumerated primary thread. The worker is terminated if the Job Object is absent, any policy field fails readback, thread enumeration is ambiguous, or the primary thread's suspend count is not exactly one. Explicit restricted-token and process-mitigation policy review remains outstanding.
3. **Job Object limits.** The worker now enforces a 10-minute timeout, 64-process active-process cap, 2 GiB per-process memory cap, 4 GiB aggregate job memory cap, 80% CPU hard cap, kill-on-job-close, and no breakaway flags. The effective extended-limit structure is queried and checked before resume. Runtime adversarial tests must still prove process creation limits and breakaway denial.
4. **Filesystem policy.** Approved tasks can receive a detached Git worktree under Spindle application data. The worker can write only within that worktree and its private profile, including a private writable offline Cargo-cache copy; the original Rust toolchain and linker directories are read-only. Handle-relative operations, reparse-point defenses, and race-resistant file mutation remain outstanding.
5. **Network policy.** The current LPAC worker receives no network capabilities and Cargo runs offline. No egress broker exists; network-enabled tasks remain disabled.
6. **MCP policy.** Launch MCP stdio servers only inside the worker boundary, with an explicit tool/schema allowlist. Do not pass user environment variables or secrets by default. Require separate approval for external side effects.
7. **Permission policy.** Project read is the baseline. Project write and process execution are separately granted in the UI; external network, MCP, and parallel-agent scopes remain unused. Grants expire on application restart. The broker enforces operation-to-scope checks and records resource-tagged audit events durably in SQLite (10,000-event retention); it is not yet a path-level policy engine. An agent plan or Skill cannot elevate itself.
8. **Audit and recovery.** Task events and permission audit history persist in SQLite. The current workflow records worktree creation and validation start/result/failure. Transcript messages, ordered task events, task state transitions, and patch-application journals persist. Startup recovery marks in-flight runs interrupted and classifies incomplete patch applications. Atomic multi-file replacement under every storage failure and idempotent worktree cleanup remain outstanding.

A Windows Job Object controls process lifetime and resource usage; it is not sufficient by itself to provide AppContainer filesystem or network isolation.

## Filesystem and Git model

The planned mutation flow is:

1. Capture the selected project revision and working tree status.
2. Create an isolated task worktree or verified copy.
3. Apply patches only inside that task root.
4. Validate every requested path against the task root at the handle level.
5. Show a diff and validation report before applying changes to the user's working tree.
6. On approval, apply the patch with expected-revision checks; otherwise discard the task workspace.
7. Preserve checkpoints so an interrupted task can resume or roll back without silently overwriting external edits.

The current Workspace editor routes manual changes through the permission broker and a temporary WorkspaceManager. Changes require a diff review and explicit apply; guarded undo refuses to overwrite later edits. Proposal checkpoints remain bounded process memory; permission audit events and task events are durable SQLite records. Git status/history and detached task worktrees are available for approved tasks. The worker currently validates only fixed Cargo commands. The model loop and its file tools still execute in the main application process and do not run inside this OS worker. The file service is not yet handle-relative or race-resistant against Windows reparse-point/TOCTOU cases. These are separate blockers; Job Object hardening does not close either one.

## Network and MCP model

MCP is an extension protocol, not a permission system. A tool's presence in an MCP server does not authorize it. Spindle must classify each tool by capability and side effect, require user approval for consequential operations, and enforce the approved scope in the worker.

Secrets should be injected per task and per tool only after explicit authorization. Environment inheritance is denied by default. Network access must use a controlled egress proxy with an auditable allowlist.

## Required Windows acceptance tests

- Attempt parent traversal, absolute, UNC, drive-prefix, alternate data stream, junction, symlink, and reparse-point escapes.
- Confirm user profile, Spindle database, model library, browser credentials, and unrelated project roots are inaccessible.
- Confirm no inherited handles or environment secrets reach the worker.
- Confirm the worker cannot break out of its Job Object or spawn an unbounded process tree.
- Confirm outbound network is denied by default and only broker-approved destinations work.
- Confirm cancellation kills the process tree and removes task scratch state.
- Confirm crash/restart recovery does not replay non-idempotent external actions.
- Confirm diff application detects changed base files and never overwrites an unreviewed user edit.
- Confirm MCP tools cannot exceed the scope granted to their server or task.
- Run threat-driven integration tests on supported Windows versions and verify actual access-denied results, not just unit-test policy decisions.

## Enablement rule

The bounded native tool set is enabled only through an approved task plan, active session scopes, and a detached task worktree. Model-authored edits never go directly into the registered project: the final diff requires separate user review and application. These tools remain application-process operations, not AppContainer-isolated execution. Arbitrary shell/process execution, external network, MCP process launch, and parallel agents stay disabled until their own OS-enforced worker/IPC design and Windows adversarial tests pass. Fixed Cargo validation is the only operation currently executed inside AppContainer/LPAC and requires a separate process-execution grant. Plan approval alone does not grant write or process permissions.
