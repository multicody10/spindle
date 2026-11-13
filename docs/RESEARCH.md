# Research and Competitive Analysis

Research reviewed for the foundation architecture, November 2026. This is a design synthesis of published product documentation and project repositories, not a claim to have reverse-engineered proprietary internals.

## Comparative feature matrix

| Project | Documented product or workflow strengths | Design lesson for Spindle |
|---|---|---|
| LM Studio | Model library and runtime controls; load-time configuration is distinct from per-request generation settings; per-model load defaults; local server/API | Treat a model's load profile, sampling profile, and provider capabilities as separate persisted objects |
| LM Studio Bionic | Projects, multiple sessions, background tasks, parallel session tabs, project context, local and cloud model choices | Keep project state distinct from session/task state; surface parallel activity without sharing conversation context |
| OpenCode | Provider abstraction; project configuration; permission rules for tools/resources; background subagents and separate child sessions; MCP tools | Give child agents scoped task/session identities and independent permissions; tool rules must be enforced in a backend policy layer |
| Cline | Explicit tool approval; plan/act modes; MCP extension support; model/provider configuration | Separate planning from authorized action; approvals should display the exact proposed tool operation and affected resources |
| Goose | Provider selection; extension system using built-in, stdio, and HTTP-based MCP transports; configurable tool exposure and timeouts | Extensions need explicit trust, tool allowlists, lifecycle limits, and provider-specific configuration |
| OpenHands | Frontend/backend separation; event stream; storage and LLM provider layers; runtime interface with Docker/local/remote execution options | Keep UI, orchestration, and execution infrastructure separate; make the runtime an explicit replaceable boundary |
| Aider | Git-aware edit workflows; architect/editor mode can split planning and code editing across models; repository map for codebase context | Consider optional planner/editor roles, but account for doubled request latency and avoid overloading small local models with excessive context |
| Continue | Separate Chat, Plan, and Agent modes; Plan mode uses read-only tool access while Agent mode can use tools; model capability requirements | Make read-only planning a real permission profile, not just a prompt instruction; detect tool-call support before enabling agent mode |
| OpenAI Codex | OS-enforced sandbox policies and approval controls; workspace scoping and network restrictions are separate concerns | Do not approximate a sandbox with path checks, command allowlists, or a confirmation dialog; the execution boundary must be enforced by the OS |
| Ollama | Local model lifecycle/API; streaming; tool calls and structured output when supported; local model discovery | Use capability negotiation per model/runtime rather than assuming all models and compatible APIs support the same features |
| Jan | Local model management and OpenAI-compatible local endpoint | Keep local model discovery and endpoint management first-class, while still probing what a compatible endpoint actually implements |
| llama.cpp | GGUF inference; server mode; parallel slots; continuous batching; KV cache controls; context shifting; multimodal projector support | Delegate inference kernels to llama.cpp; Spindle should manage process lifecycle, resource budgets, request scheduling, telemetry, and settings translation |

## What Spindle should inherit, improve, and avoid

### Inherit conceptually

- Bionic's separation of projects and sessions, independent task history, and visible background work.
- OpenCode's scoped tool permissions and subagent session identity.
- Cline's explicit action proposals and human-readable approval flow.
- Goose's extension model, but only with a permissioned host and clear lifecycle limits.
- OpenHands' separation between interface, agent service, and execution runtime.
- Aider's change-focused Git workflow and optional planning/editor role separation.
- Continue's meaningful read-only Plan mode versus tool-capable Agent mode.
- Codex's use of OS enforcement for execution boundaries.
- LM Studio's first-class model management and comprehensive load/runtime configuration.
- llama.cpp's mature inference implementation rather than reimplementing kernels.

### Improve

- Build a capability registry per provider, model, and runtime version. A setting should be shown as applicable only when there is evidence the selected endpoint can apply it.
- Explain every optimizer recommendation with the memory estimate, reserved headroom, metadata confidence, and provider/backend constraint that drove it.
- Keep model weights shareable across compatible sessions while separately accounting for session contexts, KV allocations, and active generation slots.
- Keep per-session context and execution state independent, even when a runtime process is shared.
- Make execution modes reflect actual OS permissions and network policy, not merely different system prompts.
- Record applied runtime settings separately from desired settings, and read back the effective runtime configuration after loading.
- Treat benchmarks as user-visible and opt-in; do not recalibrate by silently consuming all VRAM or interrupting another workload.

### Avoid

- Treating OpenAI-compatible URLs as proof of complete feature compatibility.
- Treating a model's tool-call ability as an authorization capability.
- Treating path traversal filtering, workspace path prefixes, or an executable allowlist as an OS sandbox.
- Automatically enabling more concurrent sessions simply because the host reports a high logical-thread count.
- Blindly maximizing context, GPU layers, batch size, or Flash Attention.
- Excessively large repository maps or retrieved context for small local models when it degrades task performance.
- Silently applying settings that require a runtime reload.
- Claiming a benchmark or automatic optimizer has improved performance without measurements.

## Architectural conclusions

1. A project should represent a workspace and durable shared context. A session should represent one task, transcript, selected model, execution state, permissions, and resource budget.
2. Parallel sessions must not duplicate compatible model weights, but they must remain isolated in conversation state, context allocation, tool permissions, and change ownership.
3. Agent planning is not execution authorization. A model may propose an action; a permission broker must independently approve or deny it.
4. A real execution sandbox is an OS-enforced boundary. The first version should keep execution disabled until a viable Windows worker is validated.
5. Load-time runtime settings and generation settings must be separately stored, validated, surfaced, and mapped to provider capabilities.
6. Memory estimates should use available RAM/VRAM, model metadata, KV cache, buffers, context, and concurrency. File size is only a proxy for weight residency and never an exact runtime measurement.
7. The optimizer should distinguish theoretical recommendations from settings applied and measured by a live runtime.
8. Model configuration should be transparent and reversible: effective values, provenance, manual overrides, reload requirements, and provider limitations must be visible.

## Primary sources

- [Bionic projects and sessions](https://lmstudio.ai/docs/bionic/projects-sessions)
- [Bionic overview](https://lmstudio.ai/docs/bionic)
- [LM Studio model loading API](https://lmstudio.ai/docs/developer/rest/load)
- [LM Studio model load settings](https://lmstudio.ai/docs/app/api/endpoints/rest/load)
- [OpenCode permissions](https://opencode.ai/docs/permissions/)
- [OpenCode agents and background subagents](https://opencode.ai/docs/agents/)
- [Cline MCP and tools](https://docs.cline.bot/mcp/mcp-overview)
- [Goose extensions](https://block.github.io/goose/docs/getting-started/using-extensions/)
- [OpenHands architecture](https://docs.openhands.dev/openhands/usage/architecture)
- [Aider architect mode](https://aider.chat/docs/usage/modes.html)
- [Aider repository map](https://aider.chat/docs/repomap.html)
- [Continue modes and permissions](https://docs.continue.dev/ide-extensions/agent/how-to-use-agent)
- [OpenAI Codex sandboxing](https://developers.openai.com/codex/security/)
- [Ollama API](https://github.com/ollama/ollama/blob/main/docs/api.md)
- [Jan](https://jan.ai/)
- [llama.cpp server documentation](https://github.com/ggml-org/llama.cpp/tree/master/examples/server)
