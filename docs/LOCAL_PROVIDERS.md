# Local Providers and Chat

Spindle discovers and chats with Ollama, LM Studio, and its managed llama.cpp runtime.

| Provider | Fixed endpoint | Discovery | Streaming chat |
|---|---|---|---|
| Ollama | `http://127.0.0.1:11434` | `GET /api/tags` | `POST /api/chat` (NDJSON) |
| LM Studio | `http://127.0.0.1:1234` | `GET /v1/models` | `POST /v1/chat/completions` (SSE) |
| llama.cpp (managed) | `http://127.0.0.1:12345` | `GET /v1/models` | `POST /v1/chat/completions` (SSE) |

Start Ollama or LM Studio yourself and load a model before selecting it in **Chat**. For llama.cpp, use the **Runtime** workspace to install a pinned CPU or Vulkan build, select an inspected GGUF file, and start the managed server. Spindle binds that server only to 127.0.0.1:12345.

## Chat behavior

- Chat supports streamed text responses from the selected local model.
- Conversation history is held in the current application window only. It is not persisted to disk or added to telemetry.
- Stop generation cancels the in-flight request and retains any partial assistant response.
- The request forwards the current context limit and generation settings. Each provider only applies the options it supports; this is not a readback of the provider's actual runtime settings.
- The displayed timing and token statistics are based on fields reported by the provider. Missing metrics remain unavailable rather than being fabricated.

## Network and parsing boundaries

- Requests are sent only to fixed IPv4 loopback endpoints above. There is no user-configurable URL.
- llama.cpp binaries are pinned to build b12347 and the downloaded ZIP is verified against the expected SHA-256 digest before extraction.
- ZIP entries are bounded by count and uncompressed size; path traversal and symbolic links are rejected.
- The process manager launches the verified executable directly with a fixed argument allowlist, a local GGUF path, and a loopback-only bind address. It never invokes a shell.
- Proxy configuration is disabled and redirects are not followed.
- Provider discovery JSON is limited to 1 MiB per response.
- Chat request history is limited to 64 messages, 65,536 bytes per message, and 256 KiB combined.
- The requested model is checked against the provider's current model inventory before generation.
- Generation settings and context values are validated again in Rust.
- Individual streaming frames are limited to 1 MiB and the complete streamed response is limited to 64 MiB.
- The HTTP request has a 15-minute total timeout. Stop generation also cancels the pending request and response stream.
- Error response bodies are bounded and surfaced as short messages. No arbitrary provider URLs or external network destinations are accepted.

Discovery and chat send no model file paths. Chat prompts are sent only to the selected local inference service over loopback. Spindle does not launch or manage Ollama or LM Studio; it launches only its separately verified llama.cpp runtime from the Runtime workspace.
