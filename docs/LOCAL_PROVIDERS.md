# Local Provider Discovery

Spindle can inspect two local inference services and list the models they expose.

| Provider | Fixed endpoint | API |
|---|---|---|
| Ollama | `http://127.0.0.1:11434` | `GET /api/tags` |
| LM Studio | `http://127.0.0.1:1234` | `GET /v1/models` |

Use the **Refresh providers** action in Model Lab to recheck them. Start the provider separately; Spindle does not install, launch, stop, or configure either service.

## Network and parsing boundaries

- Requests are sent only to the fixed IPv4 loopback endpoints above.
- Proxy configuration is disabled and redirects are not followed.
- Connection timeout is 750 ms and total request timeout is 3 seconds per provider.
- Discovery response bodies are limited to 1 MiB.
- At most 100 model entries are shown per provider. Model IDs and metadata strings are length bounded.
- Discovery sends no model file paths, prompts, API keys, or conversation data.

This is inventory discovery only. It does not prove that a model is loaded, does not start inference, and does not confirm that Spindle's saved runtime settings are supported or applied by the provider. Provider capability negotiation and inference adapters remain future work.
