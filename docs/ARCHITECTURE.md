# Architecture

## Foundation layout

Spindle uses a native Rust/Tauri 2 core and a React/TypeScript/Vite interface. Rust owns hardware inspection, GGUF parsing, optimization calculations, configuration validation, and SQLite persistence. The frontend is a presentation layer that calls a narrow typed command surface through Tauri IPC.

Current request path:

React / TypeScript → typed IPC bridge → Tauri command boundary → hardware inventory, bounded GGUF parser, explainable optimizer, fixed-loopback provider discovery and streaming chat adapters, verified llama.cpp runtime installer/process manager, and SQLite persistence.

## Trust boundaries

- The frontend is not a security authority.
- The model file is treated as untrusted binary input. The GGUF parser bounds metadata counts, string lengths, and array lengths.
- The model configuration is validated again in Rust before persistence.
- SQLite lives in the operating system application data directory, not in the project folder.
- Ollama and LM Studio discovery/chat use fixed IPv4 loopback endpoints and never launch or control those external provider processes.
- The managed llama.cpp installer uses fixed official release URLs, pinned SHA-256 digests, bounded downloads, safe ZIP path checks, and per-user application-data storage.
- The process manager launches only the installed, verified llama-server.exe with typed settings and fixed 127.0.0.1 binding; it does not invoke a shell or accept arbitrary executable paths.
- Chat streams provider tokens back to the UI through scoped Tauri events; each request can be cancelled independently.
- Chat history exists only in the active window and is not persisted.
- No shell execution, workspace file editing, MCP tool hosting, or agent loop is exposed in this milestone. The llama.cpp server is not started with its agent, tools, MCP, or filesystem features.

## Planned architecture

The next architecture layers are:

1. Provider capability registry and managed runtime adapters, extending the existing Ollama and LM Studio streaming chat boundary.
2. Central inference runtime manager, with explicit weight sharing and per-session context ownership.
3. Session coordinator and project-scoped persistence.
4. Permission broker with policy evaluation independent of model output.
5. Restricted execution worker with Windows AppContainer/LPAC where compatible, plus resource and process-tree controls.
6. File operation service using handle-relative/reparse-aware operations and atomic writes.
7. Agent orchestration and verifiable tool-result state machine.
8. Secure updater and release verification.

## Model configuration precedence

The target precedence is session override, model-specific override, automatic recommendation, and provider/runtime default. The current database stores per-model, per-profile configurations and named presets. Session overrides and provider defaults are not yet implemented.

## Inference runtime

llama.cpp is the initial managed local runtime target, not a custom inference engine. Future adapters must distinguish loaded model weights, runtime configuration, session context, KV cache, and active request scheduling. Compatible sessions may share loaded weights only when their load-time configuration is compatible. Session state and KV allocations remain independently accounted for.
