# Research and Competitive Analysis

Research reviewed for the foundation architecture, November 2026.

## Competitive feature matrix

| Project | Useful design ideas | Spindle's intended response |
|---|---|---|
| LM Studio | Model library, per-model load defaults, distinct load-time and inference parameters, runtime inspection | Make model settings capability-aware and explain automatically selected values |
| LM Studio Bionic | Projects, independent task sessions, background execution, parallel tabs | Keep project context separate from conversation/task state; build session execution as a distinct coordinator |
| OpenCode | Provider abstraction, terminal-first agent workflows, project configuration | Preserve model agnosticism while enforcing backend-owned permissions |
| Cline | Explicit tool requests, approval policies, MCP extension model, plan/act workflows | Separate tool proposal from authorization; policy must be checked outside model reasoning |
| Goose | Modular MCP extensions and local developer workflows | Use a constrained extension host with explicit trust levels |
| OpenHands | Decoupled UI and agent/server/runtime boundaries | Separate presentation, orchestration, and execution workers |
| Aider | Git-aware edit loops and repository context | Preserve focused changes, diff awareness, and verification |
| Continue | Tool handshake and policy-controlled agent mode | Use explicit tool capability metadata and read-only planning mode |
| Codex | OS-enforced sandboxing, approval controls, default network restrictions | Treat operating-system isolation as a prerequisite for execution |
| Ollama | Local model lifecycle, streaming API, tools, structured output | Use provider-specific capability detection rather than assuming compatibility |
| Jan | Local model import and local OpenAI-compatible endpoint | Keep local model discovery and endpoint management first-class |
| llama.cpp | GGUF support, server mode, parallel slots, batching, context shift, cache controls | Delegate inference kernels; centralize lifecycle, scheduling, and telemetry in Rust |

## Architectural conclusions

1. The best coding agents do not treat model output as authorization. The UI may ask for an action, but a backend permission broker must decide whether it is allowed.
2. Local model management is not just a model picker. Load-time settings, generation parameters, provider capabilities, model architecture, resource state, and session context must be separate concerns.
3. Projects and sessions should not be the same entity. A project holds a workspace and shared durable context; a session holds one task, transcript, execution state, model selection, and resource budget.
4. Parallel sessions should not create duplicate model weights. A runtime manager can share one compatible model process while preserving independent per-session contexts and KV allocations.
5. Memory estimation should be explainable and conservative. Model file size alone is insufficient; KV cache, compute buffers, available memory, context, sequence slots, and backend behavior all matter.
6. Model settings must map to actual provider capabilities. A generic OpenAI-compatible URL is not proof that every OpenAI option, tool call, or reasoning parameter is implemented.
7. Security and updater signing are separate trust boundaries. The updater must reject unsigned or invalid packages; Windows Authenticode is a separate publisher identity and SmartScreen concern.

## Primary sources

- Bionic projects and sessions: https://lmstudio.ai/docs/bionic/projects-sessions
- Bionic overview: https://lmstudio.ai/docs/bionic
- LM Studio model loading API: https://lmstudio.ai/docs/developer/rest/load
- LM Studio model load settings: https://lmstudio.ai/docs/app/api/endpoints/rest/load
- llama.cpp server documentation: https://github.com/ggml-org/llama.cpp/tree/master/examples/server
- OpenCode: https://github.com/anomalyco/opencode
- Cline: https://github.com/cline/cline
- Goose: https://github.com/block/goose
- OpenHands: https://github.com/OpenHands/OpenHands
- Aider: https://github.com/Aider-AI/aider
- Continue: https://github.com/continuedev/continue
- OpenAI Codex: https://github.com/openai/codex
- Ollama API: https://github.com/ollama/ollama/blob/main/docs/api.md
- Jan: https://jan.ai/
