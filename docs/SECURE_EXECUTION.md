# Secure agent execution design

Status: the supervised agent workflow, task worktrees, durable transcripts/events, cancellation/resume, final diff review, and journaled patch apply/undo are implemented. The fixed Cargo validation worker now starts suspended, is assigned to a Job Object, applies and reads back active-process, per-process memory, aggregate job-memory, CPU, kill-on-close, and no-breakaway policy, and only then resumes its primary thread. This code still requires Windows CI and adversarial runtime acceptance before the containment claims are considered verified. The workspace file broker remains path-based and is not race-resistant.

**Important execution boundary:** the model loop and its typed workspace tools currently execute inside the main Spindle application process. Their writes are scoped by the application permission broker and detached task worktree, but are not executed inside the AppContainer worker. Only the fixed Cargo validation commands run in AppContainer/LPAC today. This is a production blocker for treating model-authored file operations as OS-isolated execution.

## Trust boundary

Spindle's UI and Rust application process are the agent orchestrator, or agent head. They retain project identity, session history, plans, permissions, provider configuration, and task events.

Model-generated code and commands must run in a separate worker process with a distinct security boundary. The worker is the agent's hands. It must not inherit the authority of the application process.

A permission broker alone is not a sandbox. Prompt instructions, path filtering, canonicalization, command allowlists, and process timeouts are useful defense in depth, but cannot contain arbitrary hostile code.

## Required Windows worker properties

Before enabling command or autonomous file-write tools, implement and validate all of the following:

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

Until the Windows worker passes adversarial tests and the model tool host is moved behind an OS-enforced boundary, Spindle must keep arbitrary shell/process execution, network tools, MCP process launch, and parallel subagent execution disabled. Model-authored file tools remain application-process operations scoped to detached task worktrees; they must not be represented as AppContainer-isolated. The fixed Cargo validation commands are the only operations currently executed inside AppContainer/LPAC and require a separate process-execution grant. Plan approval does not itself grant execution permission.
