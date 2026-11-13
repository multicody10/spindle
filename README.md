# Spindle

**A local-first AI development environment for Windows.**

Spindle is the successor project to Rivet, designed around a Rust/Tauri application core, model-agnostic provider architecture, hardware-aware model configuration, and security boundaries that do not rely on model compliance.

> **Current status:** Foundation milestone. The desktop shell, GGUF metadata inspection, Windows hardware inventory, explainable configuration recommendations, SQLite persistence, local Ollama/LM Studio discovery, streamed local chat through Ollama, LM Studio, and the managed llama.cpp runtime, and signed Windows releases are implemented. A permission-gated workspace browser/editor is implemented. Agent orchestration, persistent sessions, and OS-level command isolation are not implemented yet.

## What works in this foundation

- Inspect GGUF v2/v3 headers and selected architecture metadata without loading model weights.
- Detect logical CPU threads and Windows system RAM; enumerate NVIDIA GPU memory when the vendor utility is installed.
- Calculate conservative llama.cpp-oriented settings using model metadata, available memory, context/KV-cache estimates, and an explicit optimization profile.
- Keep load-time runtime settings separate from generation sampling settings.
- Save per-model configurations and reusable named presets in a local SQLite database.
- Discover Ollama, LM Studio, and Spindle-managed llama.cpp endpoints, list available models, and stream local chat responses.
- Install pinned Windows x64 llama.cpp CPU, Vulkan, or NVIDIA CUDA 12.4 builds after size and SHA-256 verification, then launch and stop a GGUF model through a managed loopback server.
- Grant a project directory to a temporary workspace session with read-only access by default; optionally grant file writes and use bounded file browsing, text search, and editing.
- Validate configuration ranges and chat requests in the Rust backend.
- Chat locally with Ollama or LM Studio, including streamed responses, cancellation, and provider-reported token statistics.
- Use the bundled Roboto typeface and supplied Spindle thimble/thread wordmark.
- Validate pull requests with frontend and Rust checks; run NSIS packaging manually and CodeQL/supply-chain scans weekly or on demand.
- Build and publish signed Windows NSIS releases with an in-app Tauri updater manifest.

## What is deliberately not claimed

This milestone manages a single llama.cpp model process and supports manually editing project files through an explicit workspace grant, but it does **not** execute agent tools, run shell commands, create coding-agent sessions, or provide a hardened sandbox. Workspace access is a narrow Rust-side IPC permission gate, not an OS-enforced sandbox. The Ollama and LM Studio processes remain externally managed. The managed llama.cpp runtime applies load-time settings to its process and reports the values exposed by /props; GPU offload and some load-time settings remain unverified. Ollama and LM Studio remain provider-managed. Chat forwards generation controls supported by the selected provider. Autonomous agent execution will not be enabled before the full permission broker and isolated worker are implemented.

## Development

Requirements:

- Windows 10/11 x64 for desktop packaging.
- Node.js 22.12+ (Node 24 recommended).
- Rust stable via rustup.
- Microsoft C++ Build Tools and WebView2 Runtime.

Commands:

- Install frontend dependencies: **npm ci**
- Type-check and build: **npm run check**
- Start the desktop app: **npm run desktop**

Rust validation:

- **cargo fmt --manifest-path src-tauri/Cargo.toml -- --check**
- **cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings**
- **cargo test --locked --manifest-path src-tauri/Cargo.toml**

Build a Windows installer with **npm run package:windows**.

## Updater signing setup

Tauri's updater requires cryptographically signed update artifacts. Do not commit the private key.

1. Generate a dedicated key pair with **npm run tauri signer generate -- -w "$HOME/.tauri/spindle.key"**.
2. Put the generated public key content into the updater pubkey field in src-tauri/tauri.conf.json, replacing the placeholder.
3. Add the private key contents as the GitHub Actions repository secret TAURI_SIGNING_PRIVATE_KEY.
4. If the key is password protected, set TAURI_SIGNING_PRIVATE_KEY_PASSWORD as a second Actions secret.
5. Back up the private key securely. Losing it prevents publishing updates accepted by installations that trust the existing public key.

The release workflow stops before packaging if the public key placeholder or signing secret remains. It never disables signature verification. Windows Authenticode signing is a separate concern and is not yet configured; unsigned installers may display SmartScreen warnings.

## Data and privacy

Model paths and configuration profiles are stored in the OS application data directory in spindle.sqlite3. Model files are opened read-only for metadata inspection. Provider discovery and chat make bounded requests only to fixed loopback endpoints; chat history is kept in memory for the current window and is not saved to disk. The managed llama.cpp executable is downloaded from a fixed official release URL, checked against a pinned SHA-256 digest, safely extracted, and launched with a fixed argument allowlist. Workspace grants live only in memory; file tools use relative paths under a canonicalized root, block common credential files, and cap reads, writes, listings, and search. The workspace file layer is not yet handle-relative or race-resistant against every Windows reparse-point/TOCTOU condition. There is no telemetry implementation and the hardware profile is not uploaded.

## Project documentation

- Architecture: docs/ARCHITECTURE.md
- Local providers and chat: docs/LOCAL_PROVIDERS.md
- Windows installation and update scope: docs/DISTRIBUTION.md
- Workspace access and file tool limits: docs/WORKSPACE_ACCESS.md
- Threat model: docs/THREAT_MODEL.md
- Model configuration: docs/MODEL_CONFIGURATION.md
- Research and competitive analysis: docs/RESEARCH.md
- Roadmap: docs/ROADMAP.md
- Security policy: SECURITY.md
- CI and validation strategy: docs/CI.md
- Contributing: CONTRIBUTING.md
