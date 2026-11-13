# Spindle

**A local-first AI development environment for Windows.**

Spindle is the successor project to Rivet, designed around a Rust/Tauri application core, model-agnostic provider architecture, hardware-aware model configuration, and security boundaries that do not rely on model compliance.

> **Current status:** Foundation milestone. The desktop shell, GGUF metadata inspection, Windows hardware inventory, explainable configuration recommendations, SQLite persistence, local Ollama/LM Studio discovery, and signed Windows releases are implemented. Live inference, agent execution, session orchestration, and OS-level command isolation are not implemented yet.

## What works in this foundation

- Inspect GGUF v2/v3 headers and selected architecture metadata without loading model weights.
- Detect logical CPU threads and Windows system RAM; enumerate NVIDIA GPU memory when the vendor utility is installed.
- Calculate conservative llama.cpp-oriented settings using model metadata, available memory, context/KV-cache estimates, and an explicit optimization profile.
- Keep load-time runtime settings separate from generation sampling settings.
- Save per-model configurations and reusable named presets in a local SQLite database.
- Discover local Ollama and LM Studio endpoints and list available models without launching provider processes.
- Validate configuration ranges in the Rust backend.
- Run frontend, Rust, CodeQL, supply-chain, and Windows NSIS package validation on Windows.
- Build and publish signed Windows NSIS releases with an in-app Tauri updater manifest.

## What is deliberately not claimed

This milestone does **not** load or run a model, generate tokens, execute shell commands, edit project files, create coding-agent sessions, or provide a hardened sandbox. The Model Lab clearly labels settings as stored recommendations until a runtime adapter applies and verifies them. Agent execution will not be enabled before the permission broker and isolated worker are implemented.

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

Model paths and configuration profiles are stored in the OS application data directory in spindle.sqlite3. Model files are opened read-only for metadata inspection. Local provider discovery makes bounded GET requests only to fixed loopback endpoints; there is no telemetry implementation and the hardware profile is not uploaded. Live inference will retain an explicit provider boundary.

## Project documentation

- Architecture: docs/ARCHITECTURE.md
- Local provider discovery: docs/LOCAL_PROVIDERS.md
- Windows installation and update scope: docs/DISTRIBUTION.md
- Threat model: docs/THREAT_MODEL.md
- Model configuration: docs/MODEL_CONFIGURATION.md
- Research and competitive analysis: docs/RESEARCH.md
- Roadmap: docs/ROADMAP.md
- Security policy: SECURITY.md
- Contributing: CONTRIBUTING.md
