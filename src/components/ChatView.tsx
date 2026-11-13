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
  modelLibraryBusy: boolean;
  managedModelPath: string;
  runtimeStatus: RuntimeStatus | null;
  onSelectManagedModel: (path: string) => Promise<void>;
  onLoadManagedModel: () => Promise<string>;
  managedModelReady: boolean;
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
  modelLibraryBusy,
  managedModelPath,
  runtimeStatus,
  onSelectManagedModel,
  onLoadManagedModel,
  managedModelReady,
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
  const managedRuntimeBusy = Boolean(runtimeStatus?.running && !managedRuntimeLoaded);
  const hasInstalledBackend = runtimeStatus?.installedBackends.some((item) => item.installed) ?? false;

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
    if (!path || sending) return;
    setProviderId("llama-cpp");
    setModelId("");
    setError(null);
    setStatus("Inspecting the GGUF and preparing its configuration…");
    try {
      await onSelectManagedModel(path);
      setStatus("Model selected. Use Load model below when you are ready to start inference.");
    } catch (reason) {
      setError("Could not prepare the selected model: " + String(reason));
      setStatus("Model selection failed.");
    }
  };

  const loadManagedModel = async () => {
    if (!managedModelPath || !managedModelReady || loadingManagedModel || sending || managedRuntimeBusy || managedRuntimeLoaded) return;
    setLoadingManagedModel(true);
    setError(null);
    setStatus(hasInstalledBackend
      ? "Loading the selected model into Spindle Runtime…"
      : "Installing a compatible backend and loading the selected model…");
    try {
      const loadedModelId = await onLoadManagedModel();
      setProviderId("llama-cpp");
      setModelId(loadedModelId);
      setStatus("Model loaded. Chat is ready.");
    } catch (reason) {
      setError(String(reason));
      setStatus("Could not load the selected model.");
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
          <p>You can still choose a local GGUF model here. Selecting it does not start llama.cpp or load the model into memory.</p>
          <div><b>Spindle Runtime</b><span>Choose any model from your indexed GGUF library, then use the Load model button in this chat. Spindle installs a compatible backend if needed and starts the model for you.</span></div>
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
            <p>Choose a model below, then press Load model. The runtime stays stopped until you explicitly start it.</p>
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
                className="chat-provider-picker"
                aria-label="Provider"
                value={providerId}
                disabled={sending}
                onChange={(event) => {
                  const value = event.target.value as ChatProviderId;
                  setProviderId(value);
                  if (value === "llama-cpp") {
                    const candidate = managedModelPath || modelLibrary[0]?.path;
                    if (candidate) void selectManagedModel(candidate);
                    else setModelId("");
                  } else {
                    setModelId(providers.find((provider) => provider.id === value)?.models[0]?.id ?? "");
                  }
                }}
              >
                <option value="ollama">Ollama</option>
                <option value="lm-studio">LM Studio</option>
                <option value="llama-cpp">Spindle Runtime</option>
              </select>
              <select
                className="chat-model-picker"
                aria-label="Model"
                value={providerId === "llama-cpp" ? managedModelPath : modelId}
                disabled={sending || (providerId === "llama-cpp"
                  ? modelLibraryBusy || modelLibrary.length === 0 || Boolean(runtimeStatus?.running)
                  : !selectedProvider?.available || selectedProvider.models.length === 0)}
                onChange={(event) => {
                  if (providerId === "llama-cpp") void selectManagedModel(event.target.value);
                  else setModelId(event.target.value);
                }}
              >
                {providerId === "llama-cpp" ? (
                  <>
                    <option value="">Choose a local GGUF model…</option>
                    {managedModelPath && !modelLibrary.some((entry) => entry.path === managedModelPath) && (
                      <option value={managedModelPath}>{managedModelPath.split(/[\\/]/).pop() ?? managedModelPath}</option>
                    )}
                    {modelLibrary.map((entry) => (
                      <option key={entry.path} value={entry.path}>
                        {entry.fileName} · {entry.source}
                      </option>
                    ))}
                  </>
                ) : (
                  <>
                    {selectedProvider?.models.map((model) => (
                      <option key={model.id} value={model.id}>{model.id}</option>
                    ))}
                    {(!selectedProvider || selectedProvider.models.length === 0) && (
                      <option value="">No models available</option>
                    )}
                  </>
                )}
              </select>
              <button
                type="button"
                className="chat-refresh"
                onClick={onRefreshProviders}
                disabled={providersBusy || sending}
                title="Refresh local providers"
                aria-label="Refresh local providers"
              >
                ↻
              </button>
              <span className="chat-provider-state">
                <span className={"status-dot " + (selectedProvider?.available || managedRuntimeLoaded ? "" : "red")} />
                {selectedProvider?.available ? selectedProvider.name : managedRuntimeLoaded ? "Model loaded" : providerId === "llama-cpp" ? "Runtime stopped" : "Offline"}
              </span>
              {providerId === "llama-cpp" && (
                <button
                  type="button"
                  className="button subtle chat-load-model"
                  onClick={() => void loadManagedModel()}
                  disabled={!managedModelReady || loadingManagedModel || sending || managedRuntimeBusy || managedRuntimeLoaded}
                  title={managedRuntimeBusy ? "Stop the currently loaded model before switching" : managedRuntimeLoaded ? "This model is already loaded" : "Install a backend if needed, then load this model"}
                >
                  {loadingManagedModel
                    ? "Loading…"
                    : managedRuntimeLoaded
                      ? "Loaded"
                      : managedRuntimeBusy
                        ? "Runtime busy"
                        : hasInstalledBackend
                          ? "Load model"
                          : "Install & load"}
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
