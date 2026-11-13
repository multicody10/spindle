# Agent architecture research and implementation decisions

Research snapshot: 2026-11-13

This document records design patterns reviewed before the Project Core implementation. It distinguishes the user's earlier Rivet coding agent from the separate rivet.dev infrastructure project.

## Reviewed systems

### The user's Rivet coding agent

Private repository: https://github.com/multicody10/Rivet

Files reviewed:

- src-tauri/src/agent.rs: staged agent loop, project/repository map, bounded context assembly, action parsing, tool result feedback, stale evidence tracking, validation and retry flow.
- src-tauri/src/tools.rs: file tools, path validation, process execution, cancellation, Git helpers, and tool dispatch.
- src-tauri/src/memory.rs: project-scoped memory categories, source metadata, retrieval ranking, repository guidance.
- src-tauri/src/skills.rs: SKILL.md discovery and lazy loading from project and global directories.

Patterns adopted:

- Keep planning and execution as separate phases.
- Make task state and tool results explicit instead of relying on a chat transcript alone.
- Bound repository discovery, file context, and model output.
- Keep project memory scoped to a project and retrieve only a small relevant subset.
- Use standard Agent Skills manifests and lazy loading.

Patterns deliberately not copied as a security guarantee:

- Path canonicalization and command allowlists are defense in depth, not an OS security boundary.
- A shell wrapper, process timeout, or model instruction cannot guarantee containment of an adversarial program.
- Spindle will not inherit Rivet's process execution path and describe it as a hardened sandbox.

### Cline

Documentation: https://docs.cline.bot/core-workflows/task-management

Cline's task history retains the initial prompt, conversation, usage, and task metadata. Its resume flow reloads the task and checks file state against checkpoints before continuing. This informed Spindle's durable per-session transcript and task records. Spindle currently persists chat messages and planning state, but does not yet checkpoint filesystem mutations because the execution worker is not enabled.

### OpenHands

Conversation API: https://docs.openhands.dev/api-reference/get-conversation

Trajectory API: https://docs.openhands.dev/api-reference/get-trajectory

OpenHands exposes conversation lifecycle state and a separate trajectory view. This informed Spindle's separation of user-facing chat messages from task events. Spindle's agent_events are append-only task lifecycle records; they are not yet a complete replayable execution event stream.

### Rivet Actors and Agent Skills (separate project)

Rivet Actors: https://github.com/rivet-dev/rivet

Rivet Agents: https://github.com/rivet-dev/agents

Rivet Skills documentation: https://rivet.dev/agents/docs/skills/

The actor model separates a durable agent head (identity, state, memory, scheduling) from the sandbox where generated code executes. Skills are discovered through small name/description metadata and loaded only when needed. This informed the future worker boundary and Spindle's bounded SKILL.md discovery. This project is distinct from the user's private Rivet coding agent above.

## Spindle implementation decisions

1. **Project is the persistence boundary.** Projects have independent sessions, and sessions have independent message streams and model selections.
2. **Messages and task events are distinct.** User-facing conversation messages are not overloaded with planner state, validation output, or future worker events.
3. **Plans are structured data.** The planner returns a bounded JSON plan with ordered steps, target paths, validation expectations, and risk labels.
4. **Approval is an explicit transition.** Approving a plan records consent to the proposed plan; it does not execute it.
5. **The planner has no tools.** It receives a bounded, read-only project snapshot from Spindle's workspace layer.
6. **Project memory is advisory.** Memory records can be stale. Current repository files take precedence.
7. **Skills do not grant permissions.** Only explicitly referenced manifests are loaded into planner context, and they remain untrusted prompt context.
8. **Provider capability claims are adapter-level.** Tool calling and structured output are reported as not implemented until the adapters actually implement and test them.
9. **Permissions are least-privilege and volatile.** A session starts with project-read scope. Other scopes require an explicit grant and expire when Spindle exits.
10. **Code execution belongs in a separate OS-enforced worker.** The current application process remains the orchestration head and does not run model-generated shell commands.

## Current gaps

- The task event stream is append-only but is not yet a full event-sourced execution engine with replayable tool calls.
- The memory retriever is lexical, not embedding-based, and does not yet invalidate each file-derived memory by content hash.
- Skills support currently discovers and loads SKILL.md; bundled resources and skill authoring UI are not implemented.
- MCP discovery/transport, tool schemas, and lifecycle management are not implemented.
- Parallel subagents, task dependency scheduling, budgets, and crash-safe worker leases are not implemented.
- There is no Windows AppContainer/LPAC executor, isolated Git worktree manager, or process/network tool.
- Runtime sharing, provider-side tool calling, structured output negotiation, and calibration benchmarks remain future work.
