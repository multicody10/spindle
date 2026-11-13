import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import type { BenchmarkRun, LocalProviderStatus, ModelConfiguration } from "../lib/types";
import "./BenchmarkView.css";

interface Props {
  providers: LocalProviderStatus[];
  configuration: ModelConfiguration;
}

function BenchmarkView({ providers, configuration }: Props) {
  const [providerId, setProviderId] = useState("ollama");
  const [modelId, setModelId] = useState("");
  const [iterations, setIterations] = useState(3);
  const [runs, setRuns] = useState<BenchmarkRun[]>([]);
  const [activeRun, setActiveRun] = useState<BenchmarkRun | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const available = useMemo(() => providers.filter((provider) => provider.available), [providers]);
  const selected = available.find((provider) => provider.id === providerId) ?? available[0] ?? null;

  const refresh = useCallback(async () => {
    try {
      setRuns(await api.listModelBenchmarks(20));
    } catch (reason) {
      setError(String(reason));
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  useEffect(() => {
    if (!selected) return;
    if (selected.id !== providerId) setProviderId(selected.id);
    if (!selected.models.some((model) => model.id === modelId)) setModelId(selected.models[0]?.id ?? "");
  }, [selected, providerId, modelId]);

  const start = async () => {
    if (!selected || !modelId || running) return;
    setRunning(true);
    setError(null);
    setActiveRun(null);
    try {
      const result = await api.runModelBenchmark(
        selected.id,
        modelId,
        configuration.runtime.contextTokens,
        configuration.generation,
        iterations
      );
      setActiveRun(result);
      await refresh();
    } catch (reason) {
      setError(String(reason));
      await refresh();
    } finally {
      setRunning(false);
    }
  };

  return (
    <section className="benchmark-view">
      <header className="benchmark-heading">
        <div><span className="eyebrow">LOCAL INFERENCE</span><h2>Model benchmarks</h2>
          <p>Run an explicit, repeatable test against a local provider. Results stay in Spindle's SQLite database.</p></div>
        <span className="local-chip"><span /> NO TELEMETRY</span>
      </header>

      <section className="panel benchmark-controls">
        <div className="projects-section-title"><div><strong>Benchmark configuration</strong><small>Uses the current context and generation configuration, with output capped at 128 tokens.</small></div><span className="planning-label">MANUAL RUN</span></div>
        <div className="benchmark-form">
          <label>PROVIDER
            <select value={selected?.id ?? providerId} onChange={(event) => {
              const next = available.find((item) => item.id === event.target.value);
              setProviderId(event.target.value);
              setModelId(next?.models[0]?.id ?? "");
            }}>
              {available.map((provider) => <option key={provider.id} value={provider.id}>{provider.name}</option>)}
            </select>
          </label>
          <label>MODEL
            <select value={modelId} onChange={(event) => setModelId(event.target.value)}>
              {(selected?.models ?? []).map((model) => <option key={model.id} value={model.id}>{model.id}</option>)}
            </select>
          </label>
          <label>ITERATIONS
            <select value={iterations} onChange={(event) => setIterations(Number(event.target.value))}>
              <option value={1}>1 sample</option>
              <option value={3}>3 samples</option>
              <option value={5}>5 samples</option>
            </select>
          </label>
        </div>
        <div className="benchmark-actions">
          <span>Sequential runs avoid overlapping inference load. Only metrics and errors are saved, not generated text.</span>
          <button className="button primary" disabled={!selected || !modelId || running} onClick={() => void start()}>
            {running ? "Benchmark running…" : "Run benchmark"}
          </button>
        </div>
        {error && <p className="benchmark-error">{error}</p>}
        {!selected && <p className="projects-empty">No local provider is currently available. Start Ollama, LM Studio, or the Spindle llama.cpp runtime.</p>}
      </section>

      {activeRun && (
        <section className="panel benchmark-result">
          <div className="projects-section-title"><div><strong>Latest result</strong><small>{activeRun.providerId} · {activeRun.modelId}</small></div>
            <span className={"task-state " + activeRun.status}>{activeRun.status}</span></div>
          <div className="benchmark-metrics">
            <article><strong>{activeRun.medianTokensPerSecond?.toFixed(2) ?? "N/A"}</strong><small>MEDIAN TOKENS / SECOND</small></article>
            <article><strong>{activeRun.medianDurationMs == null ? "N/A" : (activeRun.medianDurationMs / 1000).toFixed(2) + "s"}</strong><small>MEDIAN RESPONSE DURATION</small></article>
            <article><strong>{activeRun.samples.filter((sample) => !sample.error).length}/{activeRun.requestedIterations}</strong><small>SUCCESSFUL SAMPLES</small></article>
          </div>
        </section>
      )}

      <section className="panel benchmark-history">
        <div className="projects-section-title"><div><strong>Benchmark history</strong><small>Most recent 20 runs. Median values are calculated from successful samples.</small></div>
          <button className="button subtle" onClick={() => void refresh()}>Refresh</button></div>
        {runs.length === 0 ? <p className="projects-empty">No benchmarks recorded yet.</p> :
          <div className="benchmark-list">{runs.map((run) => (
            <button key={run.id} className={activeRun?.id === run.id ? "selected" : ""} onClick={() => setActiveRun(run)}>
              <span><strong>{run.modelId}</strong><small>{run.providerId} · {new Date(run.startedAt * 1000).toLocaleString()} · {run.contextTokens.toLocaleString()} context</small></span>
              <span className="benchmark-list-metrics"><strong>{run.medianTokensPerSecond?.toFixed(2) ?? "N/A"} tok/s</strong><small>{run.medianDurationMs == null ? "N/A" : (run.medianDurationMs / 1000).toFixed(2) + "s"} · {run.status}</small></span>
            </button>
          ))}</div>}
      </section>
    </section>
  );
}

export default BenchmarkView;
