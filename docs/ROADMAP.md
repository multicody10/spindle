# Roadmap

This roadmap records implementation status rather than implying the full product brief is already complete.

## Foundation milestone

- [x] Separate public Spindle repository.
- [x] Rust/Tauri 2 and React/TypeScript application skeleton.
- [x] Restrictive production CSP and minimal Tauri capabilities.
- [x] Bounded GGUF v2/v3 metadata inspection.
- [x] Windows RAM and NVIDIA GPU telemetry.
- [x] Explainable memory-aware optimization recommendations.
- [x] Separate runtime and generation configuration schemas.
- [x] Per-model configuration and named preset persistence in SQLite.
- [x] Configuration range validation in the Rust command boundary.
- [x] Windows NSIS per-machine installer target.
- [x] Dedicated CI and signed release workflow.
- [x] Successful Windows CI and desktop build verification.
- [x] Repository-owned updater key and GitHub Actions signing secrets.
- [x] npm and Cargo lockfiles committed and used in CI.
- [x] First signed Windows release published and installed.
- [ ] Validate the one-time per-user to per-machine installation transition and in-app update path.

## Model runtime

- [x] Discovery of local Ollama and LM Studio endpoints and model inventories.
- [x] Streamed local chat adapters for Ollama and LM Studio.
- [x] In-memory chat UI with request cancellation and provider-reported metrics.
- [x] Provider capability registry for features implemented by the current adapters; tool calling and structured output remain unavailable.
- [x] Pinned Windows x64 llama.cpp CPU, Vulkan, and CUDA 12.4 runtime installation with size and SHA-256 verification.
- [ ] Shared inference runtime manager.
- [x] Single-model llama.cpp start/stop lifecycle with health checks and bounded process logs.
- [ ] Complete applied-settings readback (initial /props readback is available for exposed settings).
- [ ] Runtime logs and memory telemetry.
- [ ] Calibration benchmarks with user consent.
- [ ] Remote OpenAI-compatible provider adapter with capability negotiation.

## Core developer workflow

- [x] Persistent projects and independent sessions in SQLite.
- [x] Temporary project workspace grants with read-only defaults and separately granted write access.
- [x] Bounded workspace directory listing, UTF-8 file reads, literal text search, and file writes.
- [x] Versioned SQLite schema for projects, sessions, messages, tasks, task steps, events, and memory.
- [x] Read-only agent planner with persisted task lifecycle and approval states; no execution worker is enabled.
- [x] Read-only project snapshot planning with bounded source inventory and structured plan validation.
- [x] Append-only task event history, ordered plan steps, and explicit approve/reject transitions.
- [ ] Diff-aware file tools, undo, and conflict detection.
- [ ] Git status/history and isolated worktrees.
- [x] Project-scoped memory records with bounded lexical retrieval; embedding and source-revision freshness ranking remain future work.
- [x] Standard SKILL.md discovery and bounded manifest loading; MCP extension host remains deferred.

## Security and reliability

- [x] Revocable in-memory workspace read/write grants.
- [x] Per-session permission broker with read-only defaults, explicit scope grants, revocation, and process-restart expiry.
- [ ] General permission broker for agent tools, execution, network, and extensions.
- [ ] Windows AppContainer/LPAC execution worker feasibility validation.
- [ ] Restricted token, Job Object and process mitigation policies.
- [ ] Handle-relative, reparse-point-safe file service and race-resistant operations.
- [ ] Network restrictions and controlled egress.
- [ ] Environment/secret redaction.
- [ ] Threat-driven integration tests on Windows.

## Distribution

- [x] NSIS per-machine installer target.
- [x] Tauri updater plugin and public endpoint configuration.
- [x] Release workflow fails closed without signing material.
- [x] Publish first signed installer and latest.json.
- [ ] Validate the initial per-user to per-machine migration, subsequent updates, and data preservation on Windows.
- [ ] Windows Authenticode signing when a certificate is available.
- [ ] Stable and opt-in beta update channels.
- [ ] SBOM and release provenance.
