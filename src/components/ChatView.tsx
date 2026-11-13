import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type {
  ChatDelta,
  ChatMessage,
  ChatProviderId,
  ChatResult,
  GenerationSettings,
  LocalProviderStatus
} from "../lib/types";

interface ChatViewProps {
  providers: LocalProviderStatus[];
  providersBusy: boolean;
  providersError: string | null;
  onRefreshProviders: () => void;
  contextTokens: number;
  generation: GenerationSettings;
}

function ChatView({
  providers,
  providersBusy,
  providersError,
  onRefreshProviders,
  contextTokens,
  generation
}: ChatViewProps) {
  const [providerId, setProviderId] = useState<ChatProviderId>("ollama");
  const [modelId, setModelId] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [status, setStatus] = useState("Select a running local provider and a loaded model.");
  const [error, setError] = useState<string | null>(null);
  const [lastResult, setLastResult] = useState<ChatResult | null>(null);
  const endOfMessages = useRef<HTMLDivElement>(null);
  const activeRequest = useRef<string | null>(null);
  const cancelRequested = useRef(false);

  const selectedProvider = providers.find((provider) => provider.id === providerId);
  const availableProviders = providers.filter((provider) => provider.available);

  useEffect(() => {
    const selected = providers.find((provider) => provider.id === providerId);
    if (!selected?.available) {
      const first = providers.find((provider) => provider.available);
      if (first) {
        setProviderId(first.id as ChatProviderId);
        setModelId(first.models[0]?.id ?? "");
      }
      return;
    }
    if (!selected.models.some((model) => model.id === modelId)) {
      setModelId(selected.models[0]?.id ?? "");
    }
  }, [providers, providerId, modelId]);

  useEffect(() => {
    endOfMessages.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [messages]);

  useEffect(() => () => {
    const current = activeRequest.current;
    if (current) void api.cancelChat(current);
  }, []);

  const appendDelta = (delta: ChatDelta, id: string) => {
    if (delta.requestId !== id || !delta.delta) return;
    setMessages((current) => {
      const lastIndex = current.length - 1;
      const last = current[lastIndex];
      if (!last || last.role !== "assistant") return current;
      const updated = [...current];
      updated[lastIndex] = { ...last, content: last.content + delta.delta };
      return updated;
    });
  };

  const send = async () => {
    const text = draft.trim();
    if (!text || sending || !selectedProvider?.available || !modelId) return;

    const id = crypto.randomUUID();
    const history: ChatMessage[] = [...messages, { role: "user", content: text }];
    setMessages([...history, { role: "assistant", content: "" }]);
    setDraft("");
    setSending(true);
    activeRequest.current = id;
    cancelRequested.current = false;
    setError(null);
    setLastResult(null);
    setStatus("Generating locally…");

    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<ChatDelta>("chat-delta", (event) => appendDelta(event.payload, id));
      const result = await api.startChat({
        requestId: id,
        providerId,
        model: modelId,
        messages: history,
        contextTokens,
        generation
      });
      setLastResult(result);
      setStatus("Response complete.");
    } catch (reason) {
      const message = String(reason);
      if (cancelRequested.current || message.includes("Generation cancelled")) {
        setStatus("Generation stopped. The partial response has been kept.");
      } else {
        setError(message);
        setStatus("The provider could not complete this request.");
      }
    } finally {
      unlisten?.();
      activeRequest.current = null;
      setSending(false);
    }
  };

  const stop = async () => {
    const current = activeRequest.current;
    if (!current) return;
    cancelRequested.current = true;
    setStatus("Stopping generation…");
    try {
      const accepted = await api.cancelChat(current);
      if (!accepted) setStatus("The request is already finishing.");
    } catch (reason) {
      setError("Could not stop generation: " + String(reason));
    }
  };

  const clear = () => {
    if (sending) return;
    setMessages([]);
    setLastResult(null);
    setError(null);
    setStatus("Chat cleared.");
  };

  const onComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void send();
    }
  };

  return (
    <section className="chat-view">
      <div className="chat-heading">
        <div>
          <span className="eyebrow">LOCAL INFERENCE</span>
          <h2>Chat with a model</h2>
          <p>Stream responses directly from Ollama or LM Studio running on this PC.</p>
        </div>
        <div className="chat-heading-actions">
          <span className="local-chip"><span /> LOOPBACK ONLY</span>
          <button className="button subtle" onClick={clear} disabled={sending || messages.length === 0}>Clear chat</button>
        </div>
      </div>

      <section className="panel chat-connection">
        <div className="chat-select">
          <label htmlFor="chat-provider">PROVIDER</label>
          <select
            id="chat-provider"
            value={providerId}
            disabled={sending || availableProviders.length === 0}
            onChange={(event) => {
              const value = event.target.value as ChatProviderId;
              setProviderId(value);
              setModelId(providers.find((provider) => provider.id === value)?.models[0]?.id ?? "");
            }}
          >
            <option value="ollama">Ollama</option>
            <option value="lm-studio">LM Studio</option>
          </select>
        </div>
        <div className="chat-select model-select">
          <label htmlFor="chat-model">MODEL</label>
          <select
            id="chat-model"
            value={modelId}
            disabled={sending || !selectedProvider?.available || selectedProvider.models.length === 0}
            onChange={(event) => setModelId(event.target.value)}
          >
            {selectedProvider?.models.map((model) => (
              <option key={model.id} value={model.id}>{model.id}</option>
            ))}
            {(!selectedProvider || selectedProvider.models.length === 0) && (
              <option value="">No models found</option>
            )}
          </select>
        </div>
        <div className="chat-provider-state">
          <span className={"status-dot " + (selectedProvider?.available ? "" : "red")} />
          <span>{selectedProvider?.available ? selectedProvider.message : "Provider offline"}</span>
        </div>
        <button className="button subtle" onClick={onRefreshProviders} disabled={providersBusy || sending}>
          {providersBusy ? "Scanning…" : "Refresh"}
        </button>
      </section>

      {availableProviders.length === 0 && (
        <section className="panel chat-help">
          <strong>No local inference server detected</strong>
          <p>Start one of these providers, load a model in it, then refresh discovery.</p>
          <div><b>Ollama</b><span>Open Ollama and make sure its local service is running. Models installed with Ollama appear automatically.</span></div>
          <div><b>LM Studio</b><span>Open the Developer tab, start the local server, and load a model in the Chat or Developer view.</span></div>
        </section>
      )}

      {providersError && <p className="provider-error">{providersError}</p>}
      {error && <div className="chat-error" role="alert">{error}</div>}

      <section className="panel chat-transcript" aria-label="Chat messages">
        {messages.length === 0 ? (
          <div className="chat-empty">
            <div className="chat-empty-mark">✳</div>
            <strong>Your local chat workspace</strong>
            <p>Choose a loaded model above and send a message. Responses stream as the model generates them.</p>
          </div>
        ) : (
          <div className="chat-message-list">
            {messages.map((message, index) => (
              <article className={"chat-message " + message.role} key={index}>
                <div className="chat-message-role">{message.role === "assistant" ? "SPINDLE" : message.role.toUpperCase()}</div>
                <div className="chat-message-content">
                  {message.content || (message.role === "assistant" && sending ? <span className="typing-indicator">Thinking…</span> : null)}
                </div>
                {message.role === "assistant" && index === messages.length - 1 && lastResult && (
                  <div className="chat-metrics">
                    {lastResult.outputTokens !== null ? lastResult.outputTokens.toLocaleString() + " tokens" : ""}
                    {lastResult.tokensPerSecond !== null ? " · " + lastResult.tokensPerSecond.toFixed(1) + " tokens/s" : ""}
                    {" · " + (lastResult.durationMs / 1000).toFixed(1) + "s"}
                  </div>
                )}
              </article>
            ))}
            <div ref={endOfMessages} />
          </div>
        )}
      </section>

      <div className="chat-composer-wrap">
        <div className="chat-status">
          <span className={"status-dot " + (error ? "red" : "")} />
          <span>{status}</span>
          <span className="chat-context">
            {providerId === "ollama" ? "Context " + contextTokens.toLocaleString() : "Context managed by LM Studio"}
            {" · Output " + generation.maxOutputTokens.toLocaleString()}
          </span>
        </div>
        <div className="chat-composer">
          <textarea
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={onComposerKeyDown}
            placeholder="Message your local model…"
            rows={3}
            maxLength={65536}
            disabled={!selectedProvider?.available || !modelId || sending}
          />
          {sending ? (
            <button className="button primary stop-button" onClick={() => void stop()}>Stop generation</button>
          ) : (
            <button className="button primary send-button" onClick={() => void send()} disabled={!draft.trim() || !selectedProvider?.available || !modelId}>
              Send <span>↗</span>
            </button>
          )}
        </div>
        <p className="chat-footnote">Messages are sent only to the selected local provider at 127.0.0.1. This chat stays in memory for this window and is not saved to disk.</p>
      </div>
    </section>
  );
}

export default ChatView;
