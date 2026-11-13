# Managed llama.cpp runtime

Spindle installs and manages pinned Windows x64 llama.cpp builds without requiring a separate Ollama or LM Studio installation.

## Supported builds

| Backend | Release asset(s) | SHA-256 |
|---|---|---|
| CPU | `llama-b12347-bin-win-cpu-x64.zip` | `476a30ac458f4881051a26c541707440de4aa744d878c739669289cbe137aa09` |
| Vulkan | `llama-b12347-bin-win-vulkan-x64.zip` | `7dccbc28c03dc929687df80894d736c9fd91c71b88537f7f1a7d2eefb85a86c5` |
| CUDA 12.4 | `llama-b12347-bin-win-cuda-12.4-x64.zip` | `990e0bb9519e26c8bbd2c0de6e01e59ec596c2f3fe7309376ec416c2be56ca3d` |
| CUDA 12.4 runtime DLLs | `cudart-llama-bin-win-cuda-12.4-x64.zip` | `8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6` |

All four assets are pinned to the official `ggml-org/llama.cpp` `b12347` release. The CUDA backend consists of two archives: the versioned llama.cpp binaries and the CUDA runtime DLL bundle. Spindle extracts both into the same managed runtime directory.

## Install and launch

1. Inspect a local GGUF file in Model Lab.
2. Review and save the model configuration.
3. Open Runtime and choose CPU, Vulkan, or NVIDIA CUDA 12.4. Spindle selects CUDA by default when the hardware inventory detects an NVIDIA GPU; the selection remains changeable.
4. Install the selected backend. Downloads are streamed to temporary files rather than buffered fully in memory. Spindle follows one HTTPS redirect only from `github.com` to `release-assets.githubusercontent.com`, checks the exact published archive size and SHA-256, rejects unsafe ZIP paths and symbolic links, and extracts into a staging directory.
5. Start the selected model. Spindle validates the configuration and model metadata again in Rust before launching `llama-server.exe`.

The CUDA package includes its runtime DLLs, so a separately installed CUDA Toolkit is not required. A compatible NVIDIA display driver is still required. The upstream b12347 Windows CUDA workflow builds the x64 CUDA 12.4 backend with `GGML_NATIVE=OFF`; its CUDA 12.4 CMake defaults include `61-virtual`, so Pascal GPUs such as the GTX 1070 (compute capability 6.1) have a PTX JIT path. The first model load can incur driver compilation time. Actual offload still depends on a compatible driver and available VRAM. See the [pinned Windows CUDA build workflow](https://github.com/ggml-org/llama.cpp/blob/b12347/.github/workflows/build-cuda-windows.yml) and [pinned CUDA architecture defaults](https://github.com/ggml-org/llama.cpp/blob/b12347/ggml/src/ggml-cuda/CMakeLists.txt). CPU and Vulkan remain available as alternatives.

The runtime binds only to `127.0.0.1:12345`. Chat discovers it as llama.cpp (Spindle) and uses its OpenAI-compatible streaming chat endpoint.

## Process and data boundaries

- The executable path is derived from Spindle's managed runtime directory. Arbitrary executable paths are not accepted.
- The process is launched directly with a fixed argument allowlist. No shell, command string, MCP server, tool mode, or filesystem tool is enabled.
- The GGUF file remains at its original path and is opened by llama.cpp. Spindle does not copy or modify the model.
- stdout/stderr are captured into a bounded in-memory ring buffer. They are not persisted.
- Stop terminates the managed server process. One loaded model process is shared across sessions; Spindle's inference manager admits one active generation per session and limits simultaneous generations to the configured parallel-slot count. Additional sessions wait in a bounded, cancellable queue, and stopping the model is blocked until active and queued requests finish or are cancelled.
- Conversation history is still sent with each request. The manager does not yet preserve or pin a reusable KV cache between turns.
- The API is loopback-only and does not require an API key because it is not exposed to the LAN.
- Applied settings are read from llama.cpp's `/props` endpoint. Only values actually exposed by that endpoint are displayed; hardware offload and some load-time options are not currently reported there.
- Ollama and LM Studio remain separate, user-managed processes.

## Download behavior and limits

GitHub's release download URL responds with an HTTP 302 redirect to its release asset CDN. The installer now permits only a single HTTPS redirect from the fixed GitHub release URL to `release-assets.githubusercontent.com`; it rejects other redirect hosts and additional redirect hops.

Each archive is downloaded to disk with a per-file size cap and pinned expected byte count, then SHA-256 verified before the ZIP is opened. CUDA downloads both archives (about 678 MiB total) and therefore needs additional free disk space for the extracted runtime and staging files. Extraction is bounded to 2 GiB and 2,048 entries per archive. The installer verifies that `llama-server.exe`, `cudart64_12.dll`, `cublas64_12.dll`, and `cublasLt64_12.dll` are present together before marking CUDA installed. Temporary downloads are removed after extraction, and the staged runtime is moved into place only after all assets verify.

## Compatibility and limitations

The runtime build is deliberately pinned for reproducibility. Updating it requires changing the version, asset names, exact sizes, and SHA-256 digests together. CPU is the compatibility fallback. Vulkan requires a working Vulkan-capable graphics driver. CUDA requires a compatible NVIDIA GPU and display driver. The managed runtime currently targets Windows x64 only.

The shared inference manager supports one loaded model and one server process across multiple sessions. It enforces per-session request exclusivity, bounded queueing, configured parallel-slot admission, and safe stop behavior. It does not yet support simultaneous model weight sets or persistent per-session KV-cache reuse, and it does not provide agent execution or workspace access.
