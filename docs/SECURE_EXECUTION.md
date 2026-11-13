# Secure agent execution design

Status: design and acceptance criteria only. No model-driven process execution is enabled in Spindle.

## Trust boundary

Spindle's UI and Rust application process are the agent orchestrator, or agent head. They retain project identity, session history, plans, permissions, provider configuration, and task events.

Model-generated code and commands must run in a separate worker process with a distinct security boundary. The worker is the agent's hands. It must not inherit the authority of the application process.

A permission broker alone is not a sandbox. Prompt instructions, path filtering, canonicalization, command allowlists, and process timeouts are useful defense in depth, but cannot contain arbitrary hostile code.

## Required Windows worker properties

Before enabling command or autonomous file-write tools, implement and validate all of the following:

1. **AppContainer or LPAC isolation.** Create the worker with an AppContainer identity and an explicit capability list. Default to no network capability and no access to user profile secrets, browser stores, application databases, or unrelated projects.
2. **Restricted process creation.** Create the worker suspended, attach it to its Job Object, apply the intended process mitigations and access token, then resume it. Do not inherit application handles or credentials.
3. **Job Object limits.** Enforce wall-clock deadlines, memory limits, process count limits, CPU policy, and kill-on-job-close. Deny breakaway unless a narrowly reviewed child-process design requires it.
4. **Filesystem policy.** Give the worker a per-task scratch directory or isolated Git worktree. Start with read-only source access and a separate writable output area. Use handle-relative operations, reject reparse-point escapes, and protect against time-of-check/time-of-use races.
5. **Network policy.** Deny outbound access by default. If a task needs network access, route it through an allowlisted broker that records destinations and requires explicit scope approval.
6. **MCP policy.** Launch MCP stdio servers only inside the worker boundary, with an explicit tool/schema allowlist. Do not pass user environment variables or secrets by default. Require separate approval for external side effects.
7. **Permission policy.** Project read is the baseline. Project write, process execution, network, MCP, and parallel-agent scopes are separately granted, visible, revocable, and expire on application restart. An agent plan or Skill cannot elevate itself.
8. **Audit and recovery.** Record append-only task events for approval, worker creation, tool request/result, cancellation, validation, and cleanup. Make worker termination and scratch cleanup idempotent.

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

The existing workspace manager is still an application-level IPC permission gate. It is not yet handle-relative or race-resistant against all Windows reparse-point/TOCTOU cases.

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

Until the Windows worker passes these tests, Spindle must keep shell/process execution, autonomous file writes, network tools, MCP process launch, and parallel subagent execution disabled. Plan approval remains a record of approval only, not permission to execute.
