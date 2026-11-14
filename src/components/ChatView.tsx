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
  LocalProviderStatus,
  ModelLibraryEntry,
  ProjectSession,
  RuntimeStatus
} from "../lib/types";

const MAX_CHAT_MESSAGES = 64;
const MAX_CHAT_MESSAGE_BYTES = 64 * 1024;
const MAX_CHAT_HISTORY_BYTES = 256 * 1024;

function boundedChatHistory(messages: ChatMessage[]): ChatMessage[] | null {
  const encoder = new TextEncoder();
  const history = messages.filter((message, index) =>
    message.content.trim().length > 0 || index === messages.length - 1
  );
  if (history.length === 0) return null;

  const lastMessage = history.at(-1);
  if (!lastMessage || encoder.encode(lastMessage.content).byteLength > MAX_CHAT_MESSAGE_BYTES) {
    return null;
  }

  for (let index = 0; index < history.length - 1;) {
    const message = history[index];
    if (!message) {
      index++;
    } else if (encoder.encode(message.content).byteLength > MAX_CHAT_MESSAGE_BYTES) {
      history.splice(index, 1);
    } else {
      index++;
    }
  }

  const totalBytes = () => history.reduce(
    (total, message) => total + encoder.encode(message.content).byteLength,
    0
  );

  while (history.length > MAX_CHAT_MESSAGES || totalBytes() > MAX_CHAT_HISTORY_BYTES) {
    const lastIndex = history.length - 1;
    let removeIndex = history.findIndex(
      (message, index) => index < lastIndex && message.role !== "system"
    );
    if (removeIndex < 0) {
      removeIndex = history.findIndex((_, index) => index < lastIndex);
    }
    if (removeIndex < 0) return null;
    history.splice(removeIndex, 1);
  }

  return history;
}

interface ChatViewProps {
  providers: LocalProviderStatus[];
  providersBusy: boolean;
  providersError: string | null;
  onRefreshProviders: () => void;
  modelLibrary: ModelLibraryEntry[];
  managedModelPath: string;
  runtimeStatus: RuntimeStatus | null;
  onSelectManagedModel: (path: string, onProgress: (message: string) => void) => Promise<string>;
  onEjectManagedModel: () => Promise<void>;
  onOpenModelManager: () => void;
  contextTokens: number;
  generation: GenerationSettings;
  session?: ProjectSession | null;
}

function ChatView({
  providers,
  providersBusy,
  providersError,
  onRefreshProviders,
  modelLibrary,
  managedModelPath,
  runtimeStatus,
  onSelectManagedModel,
  onEjectManagedModel,
  onOpenModelManager,
  contextTokens,
  generation,
  session = null
}: ChatViewProps) {
  const [providerId, setProviderId] = useState<ChatProviderId>(managedModelPath ? "llama-cpp" : "ollama");
  const [ephemeralSessionId] = useState(() => crypto.randomUUID());
  const [modelId, setModelId] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [loadingManagedModel, setLoadingManagedModel] = useState(false);
  const [status, setStatus] = useState("Choose a local model. Spindle Runtime only needs to start when you are ready to chat.");
  const [error, setError] = useState<string | null>(null);
  const [lastResult, setLastResult] = useState<ChatResult | null>(null);
  const endOfMessages = useRef<HTMLDivElement>(null);
  const activeRequest = useRef<string | null>(null);
  const cancelRequested = useRef(false);
  const assistantDraft = useRef("");
  const assistantPersisted = useRef(false);

  const selectedProvider = providers.find((provider) => provider.id === providerId);
  const availableProviders = providers.filter((provider) => provider.available);
  const managedRuntimeLoaded = Boolean(
    runtimeStatus?.running &&
    runtimeStatus.modelPath?.replace(/\\/g, "/").toLowerCase() === managedModelPath.replace(/\\/g, "/").toLowerCase()
  );
  useEffect(() => {
    const selected = providers.find((provider) => provider.id === providerId);
    if (providerId === "llama-cpp" && !selected?.available) {
      setModelId("");
      return;
    }
    if (!selected?.available) {
      setModelId("");
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
    if (providerId === "ollama" && availableProviders.length === 0 && modelLibrary.length > 0) {
      setProviderId("llama-cpp");
    }
  }, [availableProviders.length, modelLibrary.length, providerId]);

  const selectManagedModel = async (path: string) => {
    if (!path || sending || loadingManagedModel) return;
    setProviderId("llama-cpp");
    setModelId("");
    setError(null);
    setLoadingManagedModel(true);
    setStatus("Inspecting and optimizing the selected GGUF…");
    try {
      const loadedModelId = await onSelectManagedModel(path, setStatus);
      setModelId(loadedModelId);
      setStatus("Model loaded. Chat is ready.");
    } catch (reason) {
      setError("Could not prepare and load the selected model: " + String(reason));
      setStatus("Model loading failed.");
    } finally {
      setLoadingManagedModel(false);
    }
  };

  useEffect(() => {
    endOfMessages.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [messages]);

  useEffect(() => {
    let disposed = false;
    setMessages([]);
    setLastResult(null);
    setError(null);
    setStatus(session ? "Restoring saved session…" : "Select a running local provider and a loaded model.");
    if (session?.providerId) setProviderId(session.providerId);
    if (session?.modelId) setModelId(session.modelId);
    if (session) {
      void api.listSessionMessages(session.id).then((stored) => {
        if (disposed) return;
        setMessages(stored.map(({ role, content }) => ({ role, content })));
        setStatus("Session restored.");
      }).catch((reason) => {
        if (!disposed) setError("Could not restore session history: " + String(reason));
      });
    }
    return () => { disposed = true; };
  }, [session?.id]);


  useEffect(() => () => {
    const current = activeRequest.current;
    if (current) void api.cancelChat(current);
  }, []);

  const appendDelta = (delta: ChatDelta, id: string) => {
    if (delta.requestId !== id || !delta.delta) return;
    assistantDraft.current += delta.delta;
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
    const transcript: ChatMessage[] = [...messages, { role: "user", content: text }];
    const history = boundedChatHistory(transcript);
    if (!history) {
      setError("This message exceeds the local chat request limit. Shorten it and try again.");
      return;
    }
    setMessages([...transcript, { role: "assistant", content: "" }]);
    assistantDraft.current = "";
    assistantPersisted.current = false;
    setDraft("");
    setSending(true);
    activeRequest.current = id;
    cancelRequested.current = false;
    setError(null);
    setLastResult(null);
    setStatus("Generating locally…");

    let unlisten: (() => void) | undefined;
    try {
      if (session) {
        await api.updateSessionModel(session.id, providerId, modelId);
        await api.appendSessionMessage(session.id, "user", text);
      }
      unlisten = await listen<ChatDelta>("chat-delta", (event) => appendDelta(event.payload, id));
      const result = await api.startChat({
        requestId: id,
        sessionId: session?.id ?? ephemeralSessionId,
        providerId,
        model: modelId,
        messages: history,
        contextTokens,
        generation
      });
      setLastResult(result);
      if (session && result.content.trim()) {
        try {
          await api.appendSessionMessage(session.id, "assistant", result.content);
          assistantPersisted.current = true;
        } catch (reason) {
          setError("The response completed but could not be saved to this session: " + String(reason));
        }
      }
      setStatus("Response complete.");
    } catch (reason) {
      if (session && !assistantPersisted.current && assistantDraft.current.trim()) {
        try {
          await api.appendSessionMessage(session.id, "assistant", assistantDraft.current);
          assistantPersisted.current = true;
        } catch (persistReason) {
          setError("The partial response could not be saved: " + String(persistReason));
        }
      }
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

  const onComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void send();
    }
  };

  return (
    <section className="chat-view">
      {availableProviders.length === 0 && (
        <section className="panel chat-help">
          <strong>No inference server is running</strong>
          <p>Choose a local GGUF model here. Spindle inspects and optimizes it, installs a compatible backend if needed, and starts it automatically.</p>
          <div><b>Spindle Runtime</b><span>Choose any model from your indexed GGUF library. Spindle inspects and optimizes it, installs a compatible backend if needed, and starts it automatically.</span></div>
          <div><b>Ollama</b><span>To use Ollama, start its local service and install a model there. Refresh discovery afterward.</span></div>
          <div><b>LM Studio</b><span>To use LM Studio, start its local server and load a model in the Chat or Developer view.</span></div>
        </section>
      )}

      {providersError && <p className="provider-error">{providersError}</p>}
      {error && <div className="chat-error" role="alert">{error}</div>}

      <section className="panel chat-transcript" aria-label="Chat messages">
        {messages.length === 0 ? (
          <div className="chat-empty">
            <div className="chat-empty-mark">✳</div>
            <strong>Your local chat workspace</strong>
            <p>Choose a local GGUF below to have Spindle prepare and start it automatically, or select an already running local provider.</p>
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
            {providerId === "ollama" ? "Context " + contextTokens.toLocaleString() : providerId === "llama-cpp" ? "Context managed by Spindle" : "Context managed by LM Studio"}
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
          <div className="chat-composer-footer">
            <div className="chat-model-controls">
              <select
                className="chat-model-picker"
                aria-label="Model"
                title="Choose a local model"
                value={providerId === "llama-cpp"
                  ? (managedModelPath ? "llama-cpp:" + managedModelPath : "")
                  : (modelId ? providerId + ":" + modelId : "")}
                disabled={sending || loadingManagedModel}
                onChange={(event) => {
                  const value = event.target.value;
                  const separator = value.indexOf(":");
                  if (separator < 0) return;
                  const nextProvider = value.slice(0, separator) as ChatProviderId;
                  const nextModel = value.slice(separator + 1);
                  if (!nextModel) return;
                  if (nextProvider === "llama-cpp") {
                    void selectManagedModel(nextModel);
                  } else {
                    setProviderId(nextProvider);
                    setModelId(nextModel);
                  }
                }}
              >
                <option value="">Choose a model…</option>
                {(modelLibrary.length > 0 || managedModelPath) && (
                  <optgroup label="Spindle Runtime · GGUF">
                    {managedModelPath && !modelLibrary.some((entry) => entry.path === managedModelPath) && (
                      <option value={"llama-cpp:" + managedModelPath}>
                        {managedModelPath.split(/[\\/]/).pop() ?? managedModelPath}
                      </option>
                    )}
                    {modelLibrary.map((entry) => (
                      <option key={entry.path} value={"llama-cpp:" + entry.path}>
                        {entry.fileName} · {entry.source}
                      </option>
                    ))}
                  </optgroup>
                )}
                {providers.filter((provider) => provider.available && provider.id !== "llama-cpp").map((provider) => (
                  <optgroup key={provider.id} label={provider.name}>
                    {provider.models.map((model) => (
                      <option key={model.id} value={provider.id + ":" + model.id}>{model.id}</option>
                    ))}
                  </optgroup>
                ))}
              </select>
              <button
                type="button"
                className="chat-refresh"
                onClick={onOpenModelManager}
                disabled={sending || loadingManagedModel}
                title="Open model library and runtime settings"
                aria-label="Open model library and runtime settings"
              >
                ⚙
              </button>
              <button
                type="button"
                className="chat-refresh"
                onClick={onRefreshProviders}
                disabled={providersBusy || sending || loadingManagedModel}
                title="Refresh local providers"
                aria-label="Refresh local providers"
              >
                ↻
              </button>
              <span className="chat-provider-state">
                <span className={"status-dot " + (selectedProvider?.available || managedRuntimeLoaded ? "" : "red")} />
                {selectedProvider?.available ? selectedProvider.name : managedRuntimeLoaded ? "Model loaded" : providerId === "llama-cpp" ? "Runtime stopped" : "Offline"}
              </span>
              {providerId === "llama-cpp" && managedRuntimeLoaded && (
                <button
                  type="button"
                  className="chat-refresh"
                  onClick={() => void onEjectManagedModel().catch((reason) => {
                    setError("Could not eject the managed model: " + String(reason));
                  })}
                  disabled={sending || loadingManagedModel}
                  title="Eject model and release memory"
                  aria-label="Eject model and release memory"
                >
                  ⏏
                </button>
              )}
            </div>
            <div className="chat-submit-controls">
              {sending ? (
                <button className="button primary stop-button" onClick={() => void stop()}>Stop</button>
              ) : (
                <button className="button primary send-button" onClick={() => void send()} disabled={!draft.trim() || !selectedProvider?.available || !modelId}>
                  Send <span>↗</span>
                </button>
              )}
            </div>
          </div>
        </div>
        <p className="chat-footnote">
          {session
            ? "This conversation is saved to the selected project session."
            : "New chats stay in memory for this window. Open a project session to save conversation history."}
        </p>
      </div>
    </section>
  );
}

export default ChatView;
