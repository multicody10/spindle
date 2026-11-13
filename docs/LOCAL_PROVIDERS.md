# Local Providers and Chat

Spindle discovers and chats with Ollama, LM Studio, and its managed llama.cpp runtime. GGUF model files can be selected from Spindle's managed library or indexed from the existing model directories used by LM Studio, GPT4All, and Jan.

| Provider | Fixed endpoint | Discovery | Streaming chat |
|---|---|---|---|
| Ollama | `http://127.0.0.1:11434` | `GET /api/tags` | `POST /api/chat` (NDJSON) |
| LM Studio | `http://127.0.0.1:1234` | `GET /v1/models` | `POST /v1/chat/completions` (SSE) |
| llama.cpp (managed) | `http://127.0.0.1:12345` | `GET /v1/models` | `POST /v1/chat/completions` (SSE) |

Spindle creates a dedicated model folder at `Documents/Spindle/Models` on first library scan. Put GGUF files there to keep a Spindle-owned library. Model Lab also indexes these existing GGUF locations:

- LM Studio: `~/.lmstudio/models/<publisher>/<model>/*.gguf`
- GPT4All: `%LOCALAPPDATA%/nomic.ai/GPT4All/*.gguf`
- Jan: `%APPDATA%/Jan/data/models/**/*.gguf`

The model roots can be overridden with `LMSTUDIO_MODELS_DIR`, `GPT4ALL_MODELS_DIR`, `JAN_MODELS_DIR`, and `KOBOLDCPP_MODELS_DIR`. KoboldCpp uses a user-selected download directory, so Spindle only scans it when that environment variable is set. External model files are indexed in place, never copied or moved. The scan is bounded and ignores symlinks and Windows reparse points. Only GGUF files are listed for direct llama.cpp inspection; Ollama-managed blobs are not treated as standalone GGUF files because Ollama already exposes those models through its local provider API.

Ollama and LM Studio must be running with a model loaded before Chat can use them. The Spindle GGUF model picker is independent of the llama.cpp server: select a local GGUF from Chat while the runtime is stopped, and Spindle will inspect the file and prepare its model configuration without loading the weights or starting a process. The picker uses the same indexed library as Settings, including Spindle's managed folder, LM Studio, GPT4All, Jan, and Hugging Face cache folders. When you are ready to chat, open **Tools → Runtime**, install a pinned CPU, Vulkan, or NVIDIA CUDA 12.4 backend if needed, and explicitly start the selected model. The CUDA installer also installs its matching runtime DLL bundle. Spindle binds the server only to 127.0.0.1:12345.

## Chat behavior

- Chat supports streamed text responses from the selected local model.
- Conversation history is held in the current application window only. It is not persisted to disk or added to telemetry.
- Stop generation cancels the in-flight request and retains any partial assistant response.
- The request forwards the current context limit and generation settings. Each provider only applies the options it supports; this is not a readback of the provider's actual runtime settings.
- The displayed timing and token statistics are based on fields reported by the provider. Missing metrics remain unavailable rather than being fabricated.

## Network and parsing boundaries

- Requests are sent only to fixed IPv4 loopback endpoints above. There is no user-configurable URL.
- llama.cpp binaries are pinned to build b12347. Downloads follow only the expected GitHub release asset redirect and are checked against pinned sizes and SHA-256 digests before extraction.
- ZIP entries are bounded by count and uncompressed size; path traversal and symbolic links are rejected.
- The process manager launches the verified executable directly with a fixed argument allowlist, a local GGUF path, and a loopback-only bind address. It never invokes a shell.
- The Ollama and LM Studio loopback clients disable proxies and redirects. The separate llama.cpp installer follows only its single, allowlisted GitHub release asset redirect, as documented in the managed runtime guide.
- Provider discovery JSON is limited to 1 MiB per response.
- Chat request history is limited to 64 messages, 65,536 bytes per message, and 256 KiB combined.
- The requested model is checked against the provider's current model inventory before generation.
- Generation settings and context values are validated again in Rust.
- Individual streaming frames are limited to 1 MiB and the complete streamed response is limited to 64 MiB.
- The HTTP request has a 15-minute total timeout. Stop generation also cancels the pending request and response stream.
- Error response bodies are bounded and surfaced as short messages. No arbitrary provider URLs or external network destinations are accepted.

Discovery and chat send no model file paths. Chat prompts are sent only to the selected local inference service over loopback. Spindle does not launch or manage Ollama or LM Studio; it launches only its separately verified llama.cpp runtime from the Runtime workspace.
