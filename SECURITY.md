# Security Policy

Spindle is public source software. Assume the implementation is inspectable and treat project files, model output, tool output, plugins, and remote responses as untrusted.

## Reporting a vulnerability

Do not publish exploit details in a public issue. Use GitHub's private vulnerability reporting feature for this repository where available. Include the affected version or commit, a concise impact description, and a minimal reproduction. Do not attach credentials, real project secrets, or private user data.

## Current security posture

The foundation build exposes a deliberately narrow Tauri command surface for read-only GGUF inspection, hardware inventory, model configuration validation, and local persistence. It does not expose a shell, filesystem write tool, browser automation, MCP host, or autonomous agent.

The frontend has no direct filesystem or process capability. Tauri capabilities grant only core defaults and updater functionality. The production CSP is restrictive and limits connections to the Tauri IPC transport, loopback model endpoints, and GitHub update hosts.

**This is not yet a security-hardened coding agent.** No isolated command worker or Windows AppContainer/LPAC policy exists in this milestone. Do not use this build to execute untrusted code on your behalf; it currently has no command execution functionality.

## Supported versions

Security fixes target the latest release on the stable channel. Prerelease builds may contain incomplete features and should not be used for sensitive projects.
