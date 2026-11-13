import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type {
  HardwareProfile,
  ModelConfiguration,
  RuntimeBackend,
  RuntimeStatus
} from "../lib/types";

interface RuntimeViewProps {
  modelPath: string;
  configuration: ModelConfiguration;
  hardware: HardwareProfile | null;
  onRuntimeChange: () => void;
}

const backendLabel = (backend: RuntimeBackend) => backend === "cpu" ? "CPU" : backend === "cuda" ? "CUDA GPU" : "Vulkan GPU";

function formatBytes(value: number): string {
  if (!value) return "Unknown";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let amount = value;
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit++;
  }
  return amount.toFixed(1) + " " + units[unit];
}

function RuntimeView({ modelPath, configuration, hardware, onRuntimeChange }: RuntimeViewProps) {
  const [backendSelection, setBackendSelection] = useState<RuntimeBackend | null>(null);
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState("Checking the managed runtime…");
  const backend = backendSelection ?? (
    hardware?.gpus.some((gpu) => gpu.vendor.toLowerCase().includes("nvidia")) ? "cuda" : "vulkan"
  );
  const selectedModelMatchesConfiguration =
    Boolean(modelPath.trim()) && configuration.modelPath === modelPath;

  const refresh = useCallback(async () => {
    try {
      const current = await api.runtimeStatus();
      setStatus(current);
      if (current.running) {
        if (current.backend) setBackendSelection(current.backend);
        setMessage("llama.cpp is running and accepting local requests.");
      } else {
        setMessage("Runtime is stopped.");
      }
    } catch (reason) {
      setError(String(reason));
      setMessage("Could not read runtime status.");
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const install = async () => {
    setBusy(true);
    setError(null);
    setMessage("Downloading and verifying the pinned llama.cpp runtime…");
    try {
      await api.installRuntime(backend);
      setMessage(backendLabel(backend) + " runtime installed.");
      await refresh();
    } catch (reason) {
      setError(String(reason));
      setMessage("Runtime installation failed.");
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const start = async () => {
    if (!modelPath.trim()) {
      setError("Inspect a GGUF model in Model Lab before starting llama.cpp.");
      return;
    }
    if (!selectedModelMatchesConfiguration) {
      setError("The selected model and its saved configuration do not match. Inspect the selected GGUF in Model Lab before loading it.");
      return;
    }
    setBusy(true);
    setError(null);
    setMessage("Starting llama.cpp and loading the selected model…");
    try {
      await api.startRuntime(modelPath, configuration, backend);
      setMessage("Model loaded. The local runtime is ready.");
      await refresh();
      onRuntimeChange();
    } catch (reason) {
      setError(String(reason));
      setMessage("The runtime could not start.");
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    setError(null);
    setMessage("Stopping the managed runtime…");
    try {
      await api.stopRuntime();
      setMessage("Runtime stopped.");
      await refresh();
      onRuntimeChange();
    } catch (reason) {
      setError(String(reason));
      setMessage("Could not stop the runtime.");
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const selectedInstall = status?.installedBackends.find((item) => item.backend === backend);
  const gpuSummary = hardware?.gpus.length
    ? hardware.gpus.map((gpu) => gpu.name + " · " + formatBytes(gpu.totalVramBytes)).join(", ")
    : "No NVIDIA GPU detected by the current hardware probe.";

  return (
    <section className="runtime-view">
      <div className="runtime-heading">
        <div>
          <span className="eyebrow">MANAGED LOCAL INFERENCE</span>
          <h2>llama.cpp runtime</h2>
          <p>Install a verified runtime, load a local GGUF model, and expose a private loopback API to Spindle Chat.</p>
        </div>
        <button className="button subtle" onClick={() => void refresh()} disabled={busy}>Refresh status</button>
      </div>

      <section className="panel runtime-status-panel">
        <div className="runtime-status-copy">
          <span className={"status-dot " + (status?.running ? "" : "red")} />
          <div>
            <strong>{status?.running ? "Runtime online" : "Runtime stopped"}</strong>
            <span>{message}</span>
          </div>
        </div>
        <div className="runtime-status-meta">
          <span>PINNED BUILD</span>
          <strong>{status?.version ?? "b12347"}</strong>
          {status?.processId && <><span>PROCESS</span><strong>{status.processId}</strong></>}
        </div>
      </section>

      {error && <div className="chat-error" role="alert">{error}</div>}

      <section className="panel runtime-panel">
        <div className="section-heading">
          <div><h3>Runtime installation</h3><p>Windows x64 release assets are pinned to an exact build and SHA-256 digest.</p></div>
        </div>
        <div className="runtime-controls">
          <label className="runtime-backend-select">
            <span>BACKEND</span>
            <select value={backend} onChange={(event) => setBackendSelection(event.target.value as RuntimeBackend)} disabled={busy || status?.running}>
              <option value="cuda">NVIDIA CUDA 12.4 GPU</option>
              <option value="vulkan">Vulkan GPU</option>
              <option value="cpu">CPU fallback</option>
            </select>
          </label>
          <div className="runtime-install-state">
            <span className={"status-dot " + (selectedInstall?.installed ? "" : "red")} />
            <span>{selectedInstall?.installed ? "Installed" : "Not installed"}</span>
          </div>
          <button className="button primary" onClick={() => void install()} disabled={busy || status?.running || selectedInstall?.installed}>
            {busy ? "Working…" : selectedInstall?.installed ? "Installed" : "Install " + backendLabel(backend)}
          </button>
        </div>
        <p className="footnote">Downloads come only from the official ggml-org/llama.cpp GitHub release. Spindle follows only GitHub’s release asset redirect, then verifies each archive’s exact size and SHA-256 before extraction. CUDA installs its matching 12.4 runtime DLL bundle too and needs about 678 MiB of downloads plus unpacked disk space.</p>
        <div className="runtime-installed-list">
          {(status?.installedBackends ?? []).map((item) => (
            <div key={item.backend}>
              <span>{backendLabel(item.backend)}</span>
              <strong>{item.installed ? "Installed" : "Not installed"}</strong>
              <small>{item.asset}</small>
            </div>
          ))}
        </div>
      </section>

      <section className="panel runtime-panel">
        <div className="section-heading">
          <div><h3>Model lifecycle</h3><p>Start one managed model process at a time. The server binds only to 127.0.0.1:12345.</p></div>
        </div>
        <div className="runtime-model-path">
          <span>SELECTED MODEL</span>
          <strong title={modelPath || "No model selected"}>{modelPath || "No GGUF model selected. Inspect one in Model Lab first."}</strong>
        </div>
        <div className="runtime-hardware">
          <span>DETECTED GPU</span>
          <strong>{gpuSummary}</strong>
          <span>CONFIGURATION</span>
          <strong>{configuration.runtime.contextTokens.toLocaleString()} context · {configuration.runtime.gpuLayers} GPU layers · {configuration.runtime.threads} CPU threads</strong>
          <span>SHARED INFERENCE SCHEDULER</span>
          <strong>
            {status?.inference.modelLoaded
              ? `${status.inference.activeRequests}/${status.inference.parallelSlots} active slots · ${status.inference.queuedRequests} queued`
              : "No managed model scheduler active"}
          </strong>
        </div>
        <div className="runtime-action-row">
          {status?.running ? (
            <button className="button stop-button" onClick={() => void stop()} disabled={busy}>Stop runtime</button>
          ) : (
            <button className="button primary" onClick={() => void start()} disabled={busy || !selectedInstall?.installed || !selectedModelMatchesConfiguration}>Load model and start</button>
          )}
          <span>{status?.running ? "Chat can now connect to llama.cpp." : selectedModelMatchesConfiguration ? "The inspected GGUF remains at its original path. Spindle passes it to llama.cpp without copying or modifying it." : "Select and inspect a GGUF model in Model Lab before loading it."}</span>
        </div>
      </section>

      {status?.appliedSettings && (
        <section className="panel runtime-panel">
          <div className="section-heading">
            <div><h3>Applied settings readback</h3><p>Values returned by llama.cpp itself, not merely the requested configuration.</p></div>
          </div>
          <div className="runtime-readback-grid">
            <div><span>MODEL</span><strong>{status.appliedSettings.modelPath ?? "Unavailable"}</strong></div>
            <div><span>CONTEXT</span><strong>{status.appliedSettings.contextTokens?.toLocaleString() ?? "Unavailable"}</strong></div>
            <div><span>TEMPERATURE</span><strong>{status.appliedSettings.temperature?.toFixed(2) ?? "Unavailable"}</strong></div>
            <div><span>TOP P</span><strong>{status.appliedSettings.topP?.toFixed(2) ?? "Unavailable"}</strong></div>
            <div><span>TOP K</span><strong>{status.appliedSettings.topK?.toLocaleString() ?? "Unavailable"}</strong></div>
            <div><span>MIN P</span><strong>{status.appliedSettings.minP?.toFixed(2) ?? "Unavailable"}</strong></div>
            <div><span>REPEAT PENALTY</span><strong>{status.appliedSettings.repeatPenalty?.toFixed(2) ?? "Unavailable"}</strong></div>
            <div><span>BUILD</span><strong>{status.appliedSettings.buildInfo ?? "Unavailable"}</strong></div>
          </div>
          <p className="footnote">{status.appliedSettings.note}</p>
        </section>
      )}

      <section className="panel runtime-panel">
        <div className="section-heading">
          <div><h3>Runtime log</h3><p>Recent process output, bounded in memory and not written to disk.</p></div>
        </div>
        <pre className="runtime-log" aria-live="polite">{status?.logs.length ? status.logs.join("\n") : "No runtime output yet."}</pre>
      </section>
    </section>
  );
}

export default RuntimeView;
