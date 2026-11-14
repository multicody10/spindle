# Coding Agent Product Research

Research date: 2026-11-13.

This comparison focuses on architecture and user workflow patterns relevant to a local-first coding agent. Product capabilities change; links below point to first-party product or documentation pages where available.

## Products reviewed

| Product | Execution and orchestration pattern | Useful design lessons for Spindle |
|---|---|---|
| **Rivet (Spindle's predecessor)** | Internal review of the original private Rivet repository. It contains a staged agent loop, typed action parsing and dispatch, bounded context assembly, cancellation, tool-result feedback, validation/retry, project memory, Skills, execution history, and repeated-operation guards. | Preserve the evidence-driven loop, bounded context, explicit dispatch, cancellation, project memory, and retry guards. Do not copy unrestricted command execution into Spindle. |
| **Cursor Agent** | Interactive codebase search and multi-file editing, terminal/tool access, diff review, checkpoints, and delegated agents. | Make changes legible, preserve checkpoints, and treat review as a first-class step. [Agent overview](https://docs.cursor.com/chat/agent) · [Checkpoints](https://docs.cursor.com/chat/checkpoints) |
| **Codex** | A harness/control plane owns sessions, tool loops, policy, approvals, event streaming, compaction, and recovery; execution is delegated to a compute environment/sandbox. | Keep model orchestration, policy, and lifecycle separate from execution. Make approval and recovery explicit. [Codex documentation](https://developers.openai.com/codex/) · [App architecture](https://developers.openai.com/codex/app/) |
| **Claude Code** | Iterative tool use with configurable permissions, bounded turns, and resumable sessions. | User-controlled permissions and explicit continuation are core lifecycle features, not peripheral UI. [Claude Code documentation](https://docs.anthropic.com/en/docs/claude-code/overview) |
| **OpenHands** | A frontend/control surface communicates with an independently managed runtime/backend where agent tools execute; runtimes can be local or isolated. | Separate UI and orchestration from the execution environment; keep the runtime replaceable. [OpenHands documentation](https://docs.all-hands.dev/) |
| **Aider** | Repository-aware edit loop with lint/test execution, Git diffs, commits, and undo workflows. | Validate after edits, use Git as a source of truth, and make change review and reversal straightforward. [Aider documentation](https://aider.chat/docs/) |
| **Rivets.dev** | Separate product, unrelated to the original Rivet predecessor. Its task model uses per-task Git worktrees/workers and normalized streaming events; review is a distinct user-facing stage. | Isolate concurrent task workspaces and stream meaningful task/change events rather than exposing only a final transcript. [Rivets.dev](https://rivets.dev/) |

## Architecture choices adopted

1. **Control plane versus worker.** Spindle's Rust/Tauri process owns task identity, permissions, provider sessions, persistence, event history, and apply/undo. The restricted worker executes only fixed validation operations.
2. **One worktree per task.** Model edits never land directly in the registered project tree. The project tree is updated only through a separate, explicit apply action.
3. **Provider-native action loop.** Ollama exposes structured chat tool calls; LM Studio and Spindle-managed llama.cpp expose OpenAI-compatible function calls. Spindle sends fixed schemas, validates streamed call fragments and arguments, routes calls through a typed permission-checked dispatcher, persists call/result records, and never executes JSON action objects embedded in ordinary assistant text.
4. **Evidence before completion.** A task cannot return completed while its final diff is truncated or has not been inspected since the last file mutation.
5. **Durable state.** Transcript messages, ordered task events, state transitions, and patch-application journals survive application restarts. Recovery inspects existing effects rather than replaying commands blindly.
6. **Skills and project memory as context only.** They improve repository-specific guidance but do not carry authority or alter permission decisions.
7. **No unsandboxed fallback.** If the restricted worker is unavailable, Spindle reports validation as unavailable; it does not silently run the requested command in the app process.

## Deliberate deviations

Spindle is local-first and security-constrained. It does not currently expose arbitrary terminal execution, remote browser access, general network access, MCP tools, hosted ephemeral VMs, or parallel agents. The agent can edit a task worktree with typed file operations, but validation is currently Cargo-only and Windows-only.

This is an intentional staged capability boundary, not feature parity with the broadest commercial agents. The next engineering focus is to close the Windows filesystem and worker security test gaps, add ecosystem-specific isolated validation profiles, strengthen transcript compaction, and improve task recovery and review UX before enabling additional tools.
