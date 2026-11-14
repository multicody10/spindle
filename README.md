# Spindle

**A local-first AI development environment for Windows.**

Spindle is the successor project to Rivet, designed around a Rust/Tauri application core, model-agnostic provider architecture, hardware-aware model configuration, and security boundaries that do not rely on model compliance.

> **Current status:** Foundation and Project Core preview. Spindle provides persistent projects and chats, deferred GGUF loading, explicit managed-runtime ejection, streamed local chat, reviewed workspace changes, durable permission auditing, Git inspection, detached task worktrees, and approved task agents using provider-native function calls. Agent tools are limited to project-scoped file, search, Git, skill, diff, and fixed Cargo-validation operations. Model edits stay in a detached task worktree; moving them into the registered project requires a separate diff review and explicit apply action. The agent tool host currently runs in the main application process behind the session permission broker and path-based workspace service, not inside AppContainer. Tool-call batches and individual results are persisted in a dedicated typed SQLite table, separate from visible chat messages. The file service is not yet handle-relative or fully race-resistant. Arbitrary shell/process execution, external network tools, MCP process launch, and parallel agents remain disabled. Restricted validation remains Cargo-only and Windows-only; Windows adversarial acceptance tests are still outstanding.

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
- Delete chats and projects with confirmation; preserve project folders and refuse deletion when a detached task worktree contains unreviewed changes or commits.
- Select a GGUF model without loading weights, load it on the first chat message, and eject the managed runtime without losing the selection.
- Display chronological chat-turn activity and optional provider-supplied reasoning separately from the saved assistant transcript.
- Execute approved tasks through provider-native structured function calls in the Ollama and OpenAI-compatible local adapters, with a fixed typed tool set in detached worktrees and durable call/result transcripts.
- Resume interrupted runs with explicit unknown-outcome tool results rather than replaying possibly completed writes; stream tool activity, require final diff review, and apply/undo patches separately.
- Generate bounded read-only project plans through Ollama, LM Studio, or Spindle-managed llama.cpp. Approval is separate from execution permissions.
- Inspect Git status/history and create detached task worktrees only for approved plans from a clean repository state.
- Run explicit Cargo format/check/test validation inside a Windows AppContainer/LPAC worker with no network capabilities, Job Object limits, offline dependencies, and a fixed command allowlist.
- Discover standard Agent Skills from project and global SKILL.md directories; explicitly referenced skills are added to planner context without adding permissions.
- Store project-scoped memory and retrieve bounded lexical matches as potentially stale planning hints.
- Apply per-session permission scopes with read-only defaults and restart expiry.
- Validate configuration ranges and chat requests in the Rust backend.
- Chat locally with Ollama or LM Studio, including streamed responses, cancellation, provider-reported token statistics, and separate reasoning streams when the selected provider exposes them.
- Use the bundled Roboto typeface and supplied Spindle thimble/thread wordmark.
- Validate pull requests with frontend and Rust checks; run NSIS packaging manually and CodeQL/supply-chain scans weekly or on demand.
- Build and publish signed Windows NSIS releases with an in-app Tauri updater manifest.
- Run user-triggered local inference benchmarks and retain metrics in SQLite without uploading prompts or generated responses.

## What is deliberately not claimed

Spindle **does** execute a bounded set of provider-native model tool calls during an approved project task. The tools are fixed and typed: project listing and reads, literal search, Agent Skills discovery, detached-worktree file edits, Git status/history/diff, and restricted Cargo validation. Each operation passes through the session permission broker; edits stay in the task worktree and the user must review and explicitly apply the final diff to the registered project. This is supervised workspace automation, not an OS-isolated sandbox. The file broker still uses path-based checks and has not been proven race-resistant against all Windows reparse-point/TOCTOU cases. The public runner does not expose arbitrary shell commands, arbitrary process launch, package installation, external network, MCP process launch, or parallel agents. The fixed Cargo format/check/test worker is the only operation currently executed in AppContainer/LPAC; its Windows adversarial acceptance suite remains outstanding. The permission broker is not a path-level ACL. The Ollama and LM Studio processes remain externally managed. The managed llama.cpp runtime applies load-time settings to its process and reports the values exposed by /props; GPU offload and some load-time settings remain unverified. Chat forwards generation controls supported by the selected provider.

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

Model paths and configuration profiles are stored in the OS application data directory in spindle.sqlite3. Model files are opened read-only for metadata inspection. Provider discovery and chat make bounded requests only to fixed loopback endpoints; chat messages and session metadata are persisted in the local SQLite database; provider-supplied reasoning activity is displayed separately and is not added to the saved conversation transcript. The managed llama.cpp executable is downloaded from a fixed official release URL, checked against a pinned SHA-256 digest, safely extracted, and launched with a fixed argument allowlist. Workspace grants and staged file-change checkpoints live only in bounded process memory. Permission audit events persist in SQLite with a global 10,000-event retention cap. File tools use relative paths under a canonicalized root, block common credential files, and cap reads, proposed writes, listings, and search. Changes are previewed and revision-checked before explicit apply; undo refuses to overwrite newer edits. The workspace file layer is not yet handle-relative or race-resistant against every Windows reparse-point/TOCTOU condition. Restricted validation uses a separate AppContainer profile and an offline Cargo cache; it does not inherit the parent application environment. There is no telemetry implementation and the hardware profile is not uploaded.

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
