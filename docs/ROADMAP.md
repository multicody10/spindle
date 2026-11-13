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
- [ ] Provider capability registry.
- [x] Pinned Windows x64 llama.cpp CPU and Vulkan runtime installation with SHA-256 verification.
- [ ] Shared inference runtime manager.
- [x] Single-model llama.cpp start/stop lifecycle with health checks and bounded process logs.
- [ ] Complete applied-settings readback (initial /props readback is available for exposed settings).
- [ ] Runtime logs and memory telemetry.
- [ ] Calibration benchmarks with user consent.
- [ ] Remote OpenAI-compatible provider adapter with capability negotiation.

## Core developer workflow

- [ ] Projects and independent sessions.
- [ ] SQLite migrations for project/session/task state.
- [ ] Agent planner and execution state machine.
- [ ] Read-only planning mode.
- [ ] Diff-aware file tools, undo, and conflict detection.
- [ ] Git status/history and isolated worktrees.
- [ ] Persistent project memory and retrieval.
- [ ] Agent Skills and MCP extension host.

## Security and reliability

- [ ] Permission broker and revocable permission grants.
- [ ] Windows AppContainer/LPAC execution worker feasibility validation.
- [ ] Restricted token, Job Object and process mitigation policies.
- [ ] Reparse-point-safe file service and race-resistant operations.
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
