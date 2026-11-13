# Local Providers and Chat

Spindle discovers and chats with two local inference services.

| Provider | Fixed endpoint | Discovery | Streaming chat |
|---|---|---|---|
| Ollama | `http://127.0.0.1:11434` | `GET /api/tags` | `POST /api/chat` (NDJSON) |
| LM Studio | `http://127.0.0.1:1234` | `GET /v1/models` | `POST /v1/chat/completions` (SSE) |

Start the provider yourself and load a model before selecting it in the **Chat** view. Use **Refresh** to update the model inventory. Spindle does not install, launch, stop, or configure either service.

## Chat behavior

- Chat supports streamed text responses from the selected local model.
- Conversation history is held in the current application window only. It is not persisted to disk or added to telemetry.
- Stop generation cancels the in-flight request and retains any partial assistant response.
- The request forwards the current context limit and generation settings. Each provider only applies the options it supports; this is not a readback of the provider's actual runtime settings.
- The displayed timing and token statistics are based on fields reported by the provider. Missing metrics remain unavailable rather than being fabricated.

## Network and parsing boundaries

- Requests are sent only to fixed IPv4 loopback endpoints above. There is no user-configurable URL.
- Proxy configuration is disabled and redirects are not followed.
- Provider discovery JSON is limited to 1 MiB per response.
- Chat request history is limited to 64 messages, 65,536 bytes per message, and 256 KiB combined.
- The requested model is checked against the provider's current model inventory before generation.
- Generation settings and context values are validated again in Rust.
- Individual streaming frames are limited to 1 MiB and the complete streamed response is limited to 64 MiB.
- The HTTP request has a 15-minute total timeout. Stop generation also cancels the pending request and response stream.
- Error response bodies are bounded and surfaced as short messages. No arbitrary provider URLs or external network destinations are accepted.

Discovery and chat send no model file paths. Chat prompts are sent only to the selected local inference service over loopback. The app does not currently launch or manage provider processes.
