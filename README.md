# Spindle

**A local-first AI development environment for Windows.**

Spindle is the successor project to Rivet, designed around a Rust/Tauri application core, model-agnostic provider architecture, hardware-aware model configuration, and security boundaries that do not rely on model compliance.

> **Current status:** Foundation and Project Core preview. Spindle now provides persistent projects and independent SQLite-backed sessions, saved local chat transcripts, structured read-only project planning, reviewable task plans, an append-only task event history, project memory retrieval, Agent Skill discovery, per-session permission scopes, and a provider capability registry. It still does not execute agent tools, run shell commands, host MCP servers, or provide an OS-enforced sandbox.

## What works in this foundation

- Inspect GGUF v2/v3 headers and selected architecture metadata without loading model weights.
- Detect logical CPU threads and Windows system RAM; enumerate NVIDIA GPU memory when the vendor utility is installed.
- Calculate conservative llama.cpp-oriented settings using model metadata, available memory, context/KV-cache estimates, and an explicit optimization profile.
- Keep load-time runtime settings separate from generation sampling settings.
- Save per-model configurations and reusable named presets in a local SQLite database.
- Discover Ollama, LM Studio, and Spindle-managed llama.cpp endpoints, list available models, and stream local chat responses.
- Install pinned Windows x64 llama.cpp CPU, Vulkan, or NVIDIA CUDA 12.4 builds after size and SHA-256 verification, then launch and stop a GGUF model through a managed loopback server.
- Browse GGUF models from `Documents/Spindle/Models`, LM Studio's active download folder (including legacy `.cache/lm-studio/models` installs), GPT4All Desktop/Python folders, Jan's `data/llamacpp/models` folder, and the Hugging Face Hub cache. File symlinks in Hugging Face snapshots are supported without following linked directories.
- Grant a project directory to a temporary workspace session with read-only access by default; optionally grant file writes and use bounded browsing/search plus diff preview, explicit apply/discard, and revision-guarded undo.
- Route workspace operations through the shared permission broker, which records bounded resource-tagged decisions and expires grants on application restart.
- Register projects and maintain independent sessions, saved messages, structured plans, task events, and approval state in SQLite.
- Generate bounded read-only project plans through Ollama, LM Studio, or Spindle-managed llama.cpp; approvals record intent but never execute the plan.
- Discover standard Agent Skills from project and global SKILL.md directories; explicitly referenced skills are added to planner context without adding permissions.
- Store project-scoped memory and retrieve bounded lexical matches as potentially stale planning hints.
- Apply per-session permission scopes with read-only defaults and restart expiry.
- Validate configuration ranges and chat requests in the Rust backend.
- Chat locally with Ollama or LM Studio, including streamed responses, cancellation, and provider-reported token statistics.
- Use the bundled Roboto typeface and supplied Spindle thimble/thread wordmark.
- Validate pull requests with frontend and Rust checks; run NSIS packaging manually and CodeQL/supply-chain scans weekly or on demand.
- Build and publish signed Windows NSIS releases with an in-app Tauri updater manifest.
- Run user-triggered local inference benchmarks and retain metrics in SQLite without uploading prompts or generated responses.

## What is deliberately not claimed

Spindle now manages project sessions and read-only planning, but it does **not** execute agent tools, run shell commands, start MCP processes, or provide a hardened OS sandbox. Workspace access is a narrow Rust-side IPC permission gate, not an OS-enforced sandbox. The Ollama and LM Studio processes remain externally managed. The managed llama.cpp runtime applies load-time settings to its process and reports the values exposed by /props; GPU offload and some load-time settings remain unverified. Ollama and LM Studio remain provider-managed. Chat forwards generation controls supported by the selected provider. Autonomous agent execution will not be enabled before the full permission broker and isolated worker are implemented.

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

## Model library discovery

Model Lab scans the following locations for .gguf files. Model files remain in place; Spindle only reads their metadata until you load one through the managed runtime.

| Source | Windows default |
|---|---|
| Spindle | %USERPROFILE%\Documents\Spindle\Models |
| LM Studio | Active downloadsFolder in settings.json; otherwise the selected LM Studio home plus models. Spindle recognizes the legacy %USERPROFILE%\.cache\lm-studio\models and current %USERPROFILE%\.lmstudio\models homes. |
| GPT4All Desktop | %LOCALAPPDATA%\nomic.ai\GPT4All |
| GPT4All Python | %USERPROFILE%\.cache\gpt4all |
| Jan | %APPDATA%\Jan\data\llamacpp\models, or the configured Jan data folder plus llamacpp\models. |
| Hugging Face Hub | %USERPROFILE%\.cache\huggingface\hub, including GGUF files in repository snapshots directories. |
| KoboldCpp | The directory selected in KoboldCpp's Loaded Files > Download Dir. Configure it for Spindle with KOBOLDCPP_MODELS_DIR. |

LM Studio's home pointer (%USERPROFILE%\.lmstudio-home-pointer) and settings file are read to locate a moved home or custom download folder. Jan's %APPDATA%\Jan\settings.json data_folder setting is read when present.

Optional environment overrides are LMSTUDIO_HOME, LMSTUDIO_MODELS_DIR, GPT4ALL_MODELS_DIR, JAN_DATA_DIR, JAN_MODELS_DIR, HF_HOME, HF_HUB_CACHE, and KOBOLDCPP_MODELS_DIR. HF_HUB_CACHE is the direct Hub cache path; HF_HOME is the parent folder containing hub.

Ollama is intentionally not scanned as a GGUF directory: it stores imported models in its own manifest/blob layout rather than as ordinary .gguf files. Use Spindle's Ollama provider discovery to select those models.

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
- Agent architecture research: docs/AGENT_ARCHITECTURE_RESEARCH.md
- Secure agent execution design: docs/SECURE_EXECUTION.md
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
