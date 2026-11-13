import { useCallback, useEffect, useMemo, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { api } from "./lib/api";
import {
  DEFAULT_CONFIGURATION,
  type AppInfo,
  type HardwareProfile,
  type ModelConfiguration,
  type ModelMetadata,
  type OptimizationProfile,
  type SavedPreset
} from "./lib/types";

const PROFILES: { value: OptimizationProfile; label: string; detail: string }[] = [
  { value: "automatic", label: "Automatic", detail: "Detect the model and hardware, then choose conservative defaults." },
  { value: "maximum_performance", label: "Maximum performance", detail: "Favor throughput while preserving memory headroom." },
  { value: "balanced", label: "Balanced", detail: "Balance context, throughput, and resource use." },
  { value: "memory_efficient", label: "Memory efficient", detail: "Reduce memory pressure and concurrency." },
  { value: "maximum_context", label: "Maximum context", detail: "Prioritize the largest estimated context that fits." },
  { value: "custom", label: "Custom", detail: "Manual values are preserved; optimization is paused." }
];

const formatBytes = (value: number) => {
  if (!value) return "Not detected";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let amount = value;
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit++;
  }
  return amount.toFixed(unit === 0 ? 0 : 1) + " " + units[unit];
};

const formatNumber = (value: number | null | undefined) =>
  value === null || value === undefined ? "Unknown" : new Intl.NumberFormat().format(value);

function App() {
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [hardware, setHardware] = useState<HardwareProfile | null>(null);
  const [modelPath, setModelPath] = useState("");
  const [model, setModel] = useState<ModelMetadata | null>(null);
  const [configuration, setConfiguration] = useState<ModelConfiguration>(DEFAULT_CONFIGURATION);
  const [presets, setPresets] = useState<SavedPreset[]>([]);
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("Detecting hardware…");
  const [optimizationWarnings, setOptimizationWarnings] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [update, setUpdate] = useState<Update | null>(null);
  const [updateState, setUpdateState] = useState("Update check has not run.");
  const [updateProgress, setUpdateProgress] = useState<number | null>(null);

  useEffect(() => {
    void Promise.all([api.appInfo(), api.hardware(), api.listPresets()])
      .then(([info, detected, savedPresets]) => {
        setAppInfo(info);
        setHardware(detected);
        setPresets(savedPresets);
        setMessage("Hardware profile ready.");
        if (info.updaterConfigured) {
          void runUpdateCheck(info);
        } else {
          setUpdateState("Updater signing is not configured yet.");
        }
      })
      .catch((reason: unknown) => {
        setError(String(reason));
        setMessage("Initialization failed.");
      });
  }, []);

  const runUpdateCheck = useCallback(async (info = appInfo) => {
    if (!info?.updaterConfigured) {
      setUpdateState("Updater is disabled until a project-specific signing key is configured.");
      return;
    }
    setUpdateState("Checking GitHub Releases…");
    try {
      const candidate = await check();
      if (candidate) {
        setUpdate(candidate);
        setUpdateState("Version " + candidate.version + " is available.");
      } else {
        setUpdate(null);
        setUpdateState("Spindle is up to date.");
      }
    } catch (reason) {
      setUpdateState("Update check failed: " + String(reason));
    }
  }, [appInfo]);

  const selectedProfile = useMemo(
    () => PROFILES.find((profile) => profile.value === configuration.profile) ?? {
      value: "automatic" as const, label: "Automatic", detail: "Conservative defaults."
    },
    [configuration.profile]
  );

  const inspect = async () => {
    const candidatePath = modelPath.trim();
    if (!candidatePath) {
      setError("Enter the full path to a local GGUF model.");
      return;
    }
    setBusy(true);
    setError(null);
    setMessage("Reading GGUF metadata…");
    setModel(null);
    try {
      const inspected = await api.inspectModel(candidatePath);
      setModel(inspected);
      const saved = await api.loadConfiguration(inspected.path, configuration.profile);
      if (saved) {
        setConfiguration(saved);
        setOptimizationWarnings([]);
        setMessage("Model inspected. Saved model configuration restored.");
      } else {
        const optimized = await api.optimizeModel(inspected.path, configuration.profile);
        setHardware(optimized.hardware);
        setConfiguration(optimized.configuration);
        setOptimizationWarnings(optimized.warnings);
        setMessage("Model inspected and initial recommendations calculated.");
      }
    } catch (reason) {
      setError(String(reason));
      setMessage("Model inspection failed.");
    } finally {
      setBusy(false);
    }
  };

  const optimize = async () => {
    if (!model) return;
    setBusy(true);
    setError(null);
    setMessage("Calculating a resource-aware recommendation…");
    try {
      const result = await api.optimizeModel(model.path, configuration.profile);
      setHardware(result.hardware);
      setConfiguration(result.configuration);
      setOptimizationWarnings(result.warnings);
      setMessage("Recommendation calculated. Review the rationale before saving.");
    } catch (reason) {
      setError(String(reason));
      setMessage("Optimization failed.");
    } finally {
      setBusy(false);
    }
  };

  const save = async () => {
    if (!model) return;
    setBusy(true);
    setError(null);
    try {
      await api.saveConfiguration({ ...configuration, modelPath: model.path });
      setConfiguration((current) => ({ ...current, modelPath: model.path }));
      setMessage("Configuration saved locally.");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const savePreset = async () => {
    if (!model) return;
    const name = window.prompt("Name this configuration preset:");
    if (!name?.trim()) return;
    try {
      await api.savePreset(name.trim(), { ...configuration, modelPath: model.path });
      setPresets(await api.listPresets());
      setMessage("Preset saved.");
    } catch (reason) {
      setError(String(reason));
    }
  };

  const loadPreset = (name: string) => {
    const preset = presets.find((entry) => entry.name === name);
    if (preset) setConfiguration(preset.configuration);
  };

  const installUpdate = async () => {
    if (!update) return;
    setUpdateProgress(0);
    setUpdateState("Downloading and verifying update…");
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") setUpdateProgress(0);
        if (event.event === "Progress") {
          const data = event.data as { chunkLength?: number; contentLength?: number };
          if (data.contentLength && data.contentLength > 0) {
            setUpdateProgress((value) => Math.min(99, (value ?? 0) + ((data.chunkLength ?? 0) / data.contentLength) * 100));
          }
        }
        if (event.event === "Finished") setUpdateProgress(100);
      });
      setUpdateState("Update installer launched.");
    } catch (reason) {
      setUpdateState("Update installation failed: " + String(reason));
    }
  };

  const changeRuntime = <K extends keyof ModelConfiguration["runtime"]>(
    key: K,
    value: ModelConfiguration["runtime"][K]
  ) => setConfiguration((current) => {
    const runtime = { ...current.runtime, [key]: value };
    const generation = key === "contextTokens" && typeof value === "number"
      ? { ...current.generation, maxOutputTokens: Math.min(current.generation.maxOutputTokens, value) }
      : current.generation;
    return { ...current, profile: "custom", runtime, generation };
  });

  const changeGeneration = <K extends keyof ModelConfiguration["generation"]>(
    key: K,
    value: ModelConfiguration["generation"][K]
  ) => setConfiguration((current) => ({
    ...current,
    profile: "custom",
    generation: { ...current.generation, [key]: value }
  }));

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark"><span /></div>
          <div><strong>spindle</strong><small>DEVELOPMENT ENVIRONMENT</small></div>
        </div>
        <div className="nav-caption">WORKSPACE</div>
        <button className="nav-item muted" disabled><span className="nav-icon">⌘</span> Projects <em>planned</em></button>
        <button className="nav-item muted" disabled><span className="nav-icon">◫</span> Sessions <em>planned</em></button>
        <div className="nav-caption section-gap">TOOLS</div>
        <button className="nav-item active"><span className="nav-icon">◈</span> Model Lab</button>
        <div className="sidebar-bottom">
          <div className="runtime-state"><span className="status-dot" /> Configuration core online</div>
          <div className="version-line">Spindle {appInfo?.version ?? "—"}</div>
        </div>
      </aside>

      <main className="main">
        <header className="topbar">
          <div>
            <div className="eyebrow">MODEL MANAGEMENT</div>
            <h1>Model Lab</h1>
          </div>
          <div className="top-actions">
            <span className="local-chip"><span /> LOCAL FIRST</span>
            <button className="button subtle" onClick={() => void runUpdateCheck()} disabled={!appInfo?.updaterConfigured}>Check updates</button>
          </div>
        </header>

        <div className="content">
          <section className="notice warning">
            <strong>Foundation build</strong>
            <span>GGUF inspection, hardware profiling, optimization recommendations, and local configuration storage are implemented. Live inference and agent execution are not connected yet. Saved values are recommendations, not active llama.cpp runtime settings.</span>
          </section>

          <section className="section-heading">
            <div><span className="eyebrow">01 / HARDWARE PROFILE</span><h2>Know the machine</h2></div>
            <span className="section-meta">Detected locally · never uploaded</span>
          </section>

          <div className="hardware-grid">
            <article className="metric-card">
              <div className="metric-label">PROCESSOR</div>
              <div className="metric-main">{hardware?.cpuName ?? "Detecting…"}</div>
              <div className="metric-foot">{hardware?.logicalThreads ?? "—"} logical threads · {hardware?.physicalCores ?? "physical core count unavailable"}</div>
            </article>
            <article className="metric-card">
              <div className="metric-label">SYSTEM MEMORY</div>
              <div className="metric-main">{formatBytes(hardware?.ramTotalBytes ?? 0)}</div>
              <div className="metric-foot">{formatBytes(hardware?.ramAvailableBytes ?? 0)} available at detection</div>
            </article>
            <article className="metric-card">
              <div className="metric-label">GRAPHICS</div>
              <div className="metric-main">{hardware?.gpus[0]?.name ?? "No supported GPU detected"}</div>
              <div className="metric-foot">{hardware?.gpus[0] ? formatBytes(hardware.gpus[0].totalVramBytes) + " VRAM · " + formatBytes(hardware.gpus[0].availableVramBytes) + " free" : "GPU detection is currently NVIDIA-specific"}</div>
            </article>
          </div>

          <section className="section-heading model-heading">
            <div><span className="eyebrow">02 / MODEL INSPECTION</span><h2>Inspect a GGUF model</h2></div>
            <span className="section-meta">Read-only metadata access</span>
          </section>

          <section className="panel model-panel">
            <label className="field-label" htmlFor="model-path">MODEL FILE</label>
            <div className="path-row">
              <input id="model-path" value={modelPath} onChange={(event) => setModelPath(event.target.value)} placeholder="C:\\Models\\model-Q4_K_M.gguf" spellCheck={false} />
              <button className="button primary" onClick={() => void inspect()} disabled={busy}>{busy ? "Working…" : "Inspect model"}</button>
            </div>
            <div className="status-line"><span className={error ? "status-dot red" : "status-dot"} />{error ?? message}</div>
            {model && (
              <div className="model-facts">
                <div><small>ARCHITECTURE</small><strong>{model.architecture ?? "Unknown"}</strong></div>
                <div><small>FILE SIZE</small><strong>{formatBytes(model.fileSizeBytes)}</strong></div>
                <div><small>QUANTIZATION</small><strong>{model.quantization ?? "Not identified"}</strong></div>
                <div><small>LAYERS</small><strong>{formatNumber(model.layerCount)}</strong></div>
                <div><small>MODEL CONTEXT</small><strong>{model.contextLength ? formatNumber(model.contextLength) : "Unknown"}</strong></div>
                <div><small>KV HEADS</small><strong>{formatNumber(model.keyValueHeads)}</strong></div>
                <div><small>EMBEDDING</small><strong>{formatNumber(model.embeddingLength)}</strong></div>
                <div><small>TENSORS</small><strong>{formatNumber(model.tensorCount)}</strong></div>
              </div>
            )}
          </section>

          <section className="section-heading">
            <div><span className="eyebrow">03 / OPTIMIZATION ENGINE</span><h2>Choose the tradeoff</h2></div>
            <button className="button subtle" onClick={() => void optimize()} disabled={!model || busy || configuration.profile === "custom"}>Recalculate</button>
          </section>

          <section className="panel">
            <div className="profile-grid">
              {PROFILES.filter((profile) => profile.value !== "custom").map((profile) => (
                <button
                  key={profile.value}
                  className={"profile-option " + (configuration.profile === profile.value ? "selected" : "")}
                  onClick={() => setConfiguration((current) => ({ ...current, profile: profile.value }))}
                  disabled={!model || busy}
                >
                  <span>{profile.label}</span><small>{profile.detail}</small>
                </button>
              ))}
            </div>
            <div className="recommendation-bar">
              <div><strong>{selectedProfile.label}</strong><span>{configuration.profile === "custom" ? "Manual edits are protected from automatic optimization." : configuration.optimizationNotes.length ? "Recommendation rationale available below." : "Inspect a model to calculate a recommendation."}</span></div>
              <button className="button primary" onClick={() => void optimize()} disabled={!model || busy || configuration.profile === "custom"}>Optimize configuration</button>
            </div>
            {configuration.optimizationNotes.length > 0 && (
              <ul className="reason-list">{configuration.optimizationNotes.map((note, index) => <li key={index}>{note}</li>)}</ul>
            )}
            {optimizationWarnings.length > 0 && (
              <ul className="reason-list warning-list">{optimizationWarnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul>
            )}
          </section>

          <section className="section-heading">
            <div><span className="eyebrow">04 / INFERENCE CONFIGURATION</span><h2>Model settings</h2></div>
            <div className="view-toggle"><button className={!advanced ? "chosen" : ""} onClick={() => setAdvanced(false)}>Basic</button><button className={advanced ? "chosen" : ""} onClick={() => setAdvanced(true)}>Advanced</button></div>
          </section>

          <section className="panel settings-panel">
            <div className="settings-group">
              <div className="settings-group-title"><div><strong>Runtime</strong><small>Load-time and resource allocation values</small></div><span>LLAMA.CPP TARGET</span></div>
              <div className="settings-grid">
                <label>Context length <input type="number" min="1" max={Math.min(model?.contextLength && model.contextLength > 0 ? model.contextLength : 131072, 131072)} step="1" value={configuration.runtime.contextTokens} onChange={(event) => changeRuntime("contextTokens", Number(event.target.value))} /></label>
                <label>GPU layers <input type="number" min="0" max={model?.layerCount ?? 256} value={configuration.runtime.gpuLayers} onChange={(event) => changeRuntime("gpuLayers", Number(event.target.value))} /></label>
                <label>CPU threads <input type="number" min="1" max="256" value={configuration.runtime.threads} onChange={(event) => changeRuntime("threads", Number(event.target.value))} /></label>
                <label>Batch threads <input type="number" min="1" max="256" value={configuration.runtime.batchThreads} onChange={(event) => changeRuntime("batchThreads", Number(event.target.value))} /></label>
              </div>
            </div>
            <div className="settings-group">
              <div className="settings-group-title"><div><strong>Generation</strong><small>Sampling defaults, separate from hardware optimization</small></div><span>REQUEST-TIME TARGET</span></div>
              <div className="settings-grid">
                <label>Temperature <input type="number" min="0" max="5" step="0.05" value={configuration.generation.temperature} onChange={(event) => changeGeneration("temperature", Number(event.target.value))} /></label>
                <label>Top P <input type="number" min="0" max="1" step="0.01" value={configuration.generation.topP} onChange={(event) => changeGeneration("topP", Number(event.target.value))} /></label>
                <label>Top K <input type="number" min="0" max="1000" value={configuration.generation.topK} onChange={(event) => changeGeneration("topK", Number(event.target.value))} /></label>
                <label>Maximum output tokens <input type="number" min="1" max="131072" value={configuration.generation.maxOutputTokens} onChange={(event) => changeGeneration("maxOutputTokens", Number(event.target.value))} /></label>
              </div>
            </div>
            {advanced && (
              <>
                <div className="settings-group">
                  <div className="settings-group-title"><div><strong>Advanced runtime</strong><small>Only mapped to real runtime flags after provider integration</small></div></div>
                  <div className="settings-grid">
                    <label>Evaluation batch size <input type="number" min="1" max="8192" value={configuration.runtime.batchSize} onChange={(event) => changeRuntime("batchSize", Number(event.target.value))} /></label>
                    <label>Micro batch size <input type="number" min="1" max="2048" value={configuration.runtime.microBatchSize} onChange={(event) => changeRuntime("microBatchSize", Number(event.target.value))} /></label>
                    <label>Parallel slots <input type="number" min="1" max="32" value={configuration.runtime.parallelSlots} onChange={(event) => changeRuntime("parallelSlots", Number(event.target.value))} /></label>
                    <label>KV cache K type <select value={configuration.runtime.kvCacheKType} onChange={(event) => changeRuntime("kvCacheKType", event.target.value)}><option value="f16">F16</option><option value="q8_0">Q8_0</option><option value="q4_0">Q4_0</option></select></label>
                    <label>KV cache V type <select value={configuration.runtime.kvCacheVType} onChange={(event) => changeRuntime("kvCacheVType", event.target.value)}><option value="f16">F16</option><option value="q8_0">Q8_0</option><option value="q4_0">Q4_0</option></select></label>
                    <label>Repetition penalty <input type="number" min="0.1" max="3" step="0.05" value={configuration.generation.repeatPenalty} onChange={(event) => changeGeneration("repeatPenalty", Number(event.target.value))} /></label>
                  </div>
                  <div className="toggle-list">
                    <label><input type="checkbox" checked={configuration.runtime.flashAttention} onChange={(event) => changeRuntime("flashAttention", event.target.checked)} /> Flash Attention</label>
                    <label><input type="checkbox" checked={configuration.runtime.kvCacheOnGpu} onChange={(event) => changeRuntime("kvCacheOnGpu", event.target.checked)} /> Place KV cache on GPU</label>
                    <label><input type="checkbox" checked={configuration.runtime.useMemoryMapping} onChange={(event) => changeRuntime("useMemoryMapping", event.target.checked)} /> Memory mapping</label>
                    <label><input type="checkbox" checked={configuration.runtime.lockMemory} onChange={(event) => changeRuntime("lockMemory", event.target.checked)} /> Lock model memory (requires OS permission)</label>
                  </div>
                </div>
                <div className="settings-group">
                  <div className="settings-group-title"><div><strong>More sampling controls</strong><small>Stored independently of runtime hardware settings</small></div></div>
                  <div className="settings-grid">
                    <label>Min P <input type="number" min="0" max="1" step="0.01" value={configuration.generation.minP} onChange={(event) => changeGeneration("minP", Number(event.target.value))} /></label>
                    <label>Typical P <input type="number" min="0" max="1" step="0.01" value={configuration.generation.typicalP} onChange={(event) => changeGeneration("typicalP", Number(event.target.value))} /></label>
                    <label>Repeat last N <input type="number" min="-1" max="8192" value={configuration.generation.repeatLastN} onChange={(event) => changeGeneration("repeatLastN", Number(event.target.value))} /></label>
                    <label>Presence penalty <input type="number" min="-2" max="2" step="0.05" value={configuration.generation.presencePenalty} onChange={(event) => changeGeneration("presencePenalty", Number(event.target.value))} /></label>
                    <label>Frequency penalty <input type="number" min="-2" max="2" step="0.05" value={configuration.generation.frequencyPenalty} onChange={(event) => changeGeneration("frequencyPenalty", Number(event.target.value))} /></label>
                    <label>Seed <input type="number" value={configuration.generation.seed ?? ""} placeholder="Random" onChange={(event) => changeGeneration("seed", event.target.value === "" ? null : Number(event.target.value))} /></label>
                  </div>
                </div>
              </>
            )}
            <div className="save-row">
              <div><strong>Per-model configuration</strong><small>Stored locally in Spindle's SQLite database. It does not modify the model file.</small></div>
              <div className="save-actions">
                <select aria-label="Load saved preset" defaultValue="" onChange={(event) => loadPreset(event.target.value)}><option value="">Load preset…</option>{presets.map((preset) => <option key={preset.name} value={preset.name}>{preset.name}</option>)}</select>
                <button className="button subtle" onClick={() => void savePreset()} disabled={!model}>Save preset</button>
                <button className="button primary" onClick={() => void save()} disabled={!model || busy}>Save model settings</button>
              </div>
            </div>
          </section>

          <section className="section-heading">
            <div><span className="eyebrow">05 / APPLICATION UPDATES</span><h2>Updates</h2></div>
            <span className="section-meta">Stable channel</span>
          </section>
          <section className="panel update-panel">
            <div className="update-icon">↻</div>
            <div className="update-copy"><strong>Spindle {appInfo?.version ?? "—"}</strong><span>{updateState}</span>{update?.body && <p>{update.body}</p>}{updateProgress !== null && <div className="progress"><span style={{ width: updateProgress + "%" }} /></div>}</div>
            {update ? <button className="button primary" onClick={() => void installUpdate()}>Install update</button> : <button className="button subtle" onClick={() => void runUpdateCheck()} disabled={!appInfo?.updaterConfigured}>Check</button>}
          </section>
          {!appInfo?.updaterConfigured && (
            <p className="footnote">The updater remains disabled until a project-specific Tauri updater key is generated and its private half is installed as a protected GitHub Actions secret. Signature verification is never bypassed.</p>
          )}
          <footer>SPINDLE · LOCAL-FIRST DEVELOPMENT ENVIRONMENT <span>{hardware?.os ?? "DESKTOP"}</span></footer>
        </div>
      </main>
    </div>
  );
}

export default App;
