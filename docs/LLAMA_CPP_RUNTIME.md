# Managed llama.cpp runtime

Spindle can install and manage a pinned Windows x64 build of llama.cpp without requiring a separate Ollama or LM Studio installation.

## Supported builds

| Backend | Asset | SHA-256 |
|---|---|---|
| CPU | llama-b12347-bin-win-cpu-x64.zip | 476a30ac458f4881051a26c541707440de4aa744d878c739669289cbe137aa09 |
| Vulkan | llama-b12347-bin-win-vulkan-x64.zip | 7dccbc28c03dc929687df80894d736c9fd91c71b88537f7f1a7d2eefb85a86c5 |

Both archives are pinned to the official ggml-org/llama.cpp release tag b12347. Spindle does not resolve a moving latest URL at install time.

## Install and launch

1. Inspect a local GGUF file in Model Lab.
2. Review and save the model configuration.
3. Open Runtime and choose CPU or Vulkan.
4. Install the selected backend. Spindle downloads the fixed official release asset, enforces a download size limit, verifies SHA-256, rejects unsafe ZIP paths and symbolic links, and extracts into the per-user application data directory.
5. Start the selected model. Spindle validates the configuration and model metadata again in Rust before launching llama-server.exe.

The runtime binds only to 127.0.0.1:12345. Chat discovers it as llama.cpp (Spindle) and uses its OpenAI-compatible streaming chat endpoint.

## Process and data boundaries

- The executable path is derived from Spindle's managed runtime directory. Arbitrary executable paths are not accepted.
- The process is launched directly with a fixed argument allowlist. No shell, command string, MCP server, tool mode, or filesystem tool is enabled.
- The GGUF file remains at its original path and is opened by llama.cpp. Spindle does not copy or modify the model.
- stdout/stderr are captured into a bounded in-memory ring buffer. They are not persisted.
- Stop terminates the managed server process. One model process is supported at a time.
- The API is loopback-only and does not require an API key because it is not exposed to the LAN.
- Applied settings are read from llama.cpp's /props endpoint. Only values actually exposed by that endpoint are displayed; GPU offload and some load-time options are not currently reported there.
- Ollama and LM Studio remain separate, user-managed processes.

## Compatibility and limitations

The runtime build is deliberately pinned for reproducibility. Updating it requires changing the version, asset names, and independently verified SHA-256 digests together. CPU is the compatibility fallback. Vulkan requires a working Vulkan-capable graphics driver. The managed runtime currently targets Windows x64 only.

The current runtime manager supports one loaded model and one server process. It is not yet the shared multi-session inference manager, and it does not provide agent execution or workspace access.
