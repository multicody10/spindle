import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type {
  ChatDelta,
  ChatThinkingDelta,
  ChatMessage,
  ChatProviderId,
  ChatResult,
  GenerationSettings,
  LocalProviderStatus,
  ModelLibraryEntry,
  ProjectSession,
  RuntimeStatus
} from "../lib/types";

const MODEL_SELECTION_KEY = "spindle.model-selection.v1";

interface RememberedModelSelection {
  providerId: ChatProviderId;
  modelId: string;
  managedModelPath: string;
}

function readRememberedModel(sessionId?: string): RememberedModelSelection | null {
  try {
    const raw = sessionId
      ? localStorage.getItem(MODEL_SELECTION_KEY + "." + sessionId)
      : null;
    const saved = JSON.parse(raw ?? localStorage.getItem(MODEL_SELECTION_KEY) ?? "null") as Partial<RememberedModelSelection> | null;
    if (!saved || !["ollama", "lm-studio", "llama-cpp"].includes(saved.providerId ?? "") || typeof saved.modelId !== "string") return null;
    return {
      providerId: saved.providerId as ChatProviderId,
      modelId: saved.modelId,
      managedModelPath: typeof saved.managedModelPath === "string" ? saved.managedModelPath : ""
    };
  } catch { return null; }
}

type TurnActivityState = "active" | "complete" | "error";
interface TurnActivity {
  id: string;
  label: string;
  detail: string;
  state: TurnActivityState;
}

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
  onSelectManagedModel: (path: string, onProgress: (message: string) => void) => Promise<void>;
  onLoadManagedModel: (path: string, onProgress: (message: string) => void) => Promise<string>;
  onEjectManagedModel: () => Promise<void>;
  onOpenModelManager: () => void;
  contextTokens: number;
  generation: GenerationSettings;
  session: ProjectSession;
  onSessionTitleChange: (sessionId: string, title: string) => void;
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
  onLoadManagedModel,
  onEjectManagedModel,
  onOpenModelManager,
  contextTokens,
  generation,
  session,
  onSessionTitleChange
}: ChatViewProps) {
  const rememberedModel = readRememberedModel(session.id);
  const [providerId, setProviderId] = useState<ChatProviderId>(
    session.providerId ?? rememberedModel?.providerId ?? (managedModelPath ? "llama-cpp" : "ollama")
  );
  const [modelId, setModelId] = useState(session.modelId ?? rememberedModel?.modelId ?? "");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [loadingManagedModel, setLoadingManagedModel] = useState(false);
  const [status, setStatus] = useState("Choose a local model. Spindle Runtime only needs to start when you are ready to chat.");
  const [error, setError] = useState<string | null>(null);
  const [lastResult, setLastResult] = useState<ChatResult | null>(null);
  const [turnActivities, setTurnActivities] = useState<TurnActivity[]>([]);
  const [thinkingText, setThinkingText] = useState("");
  const thinkingDraft = useRef("");
  const endOfMessages = useRef<HTMLDivElement>(null);
  const activeRequest = useRef<string | null>(null);
  const cancelRequested = useRef(false);
  const assistantDraft = useRef("");
  const assistantPersisted = useRef(false);

  const addTurnActivity = (label: string, detail: string, state: TurnActivityState = "complete") => {
    setTurnActivities((current) => [
      ...current.map((entry) => entry.state === "active" ? { ...entry, state: "complete" as const } : entry),
      { id: crypto.randomUUID(), label, detail, state }
    ].slice(-12));
  };

  const updateActiveTurnActivity = (label: string, detail: string) => {
    setTurnActivities((current) => {
      const next = [...current];
      const last = next[next.length - 1];
      if (last?.state === "active") next[next.length - 1] = { ...last, label, detail };
      else next.push({ id: crypto.randomUUID(), label, detail, state: "active" });
      return next.slice(-12);
    });
  };

  const finishTurnActivity = (label: string, detail: string, state: TurnActivityState = "complete") => {
    setTurnActivities((current) => [
      ...current.map((entry) => entry.state === "active" ? { ...entry, state: "complete" as const } : entry),
      { id: crypto.randomUUID(), label, detail, state }
    ].slice(-12));
  };

  const selectedProvider = providers.find((provider) => provider.id === providerId);
  const availableProviders = providers.filter((provider) => provider.available);
  const selectedManagedModelPath = providerId === "llama-cpp" &&
    (!rememberedModel || rememberedModel.providerId === "llama-cpp")
    ? rememberedModel?.managedModelPath || managedModelPath
    : "";
  const managedSelectionReady = providerId === "llama-cpp" && Boolean(selectedManagedModelPath);
  const managedRuntimeLoaded = Boolean(
    runtimeStatus?.running &&
    runtimeStatus.modelPath?.replace(/\\/g, "/").toLowerCase() === selectedManagedModelPath.replace(/\\/g, "/").toLowerCase()
  );

  const rememberModel = (nextProvider: ChatProviderId, nextModel: string, modelPath = "") => {
    const selection: RememberedModelSelection = {
      providerId: nextProvider,
      modelId: nextModel,
      managedModelPath: modelPath
    };
    try {
      localStorage.setItem(MODEL_SELECTION_KEY, JSON.stringify(selection));
      localStorage.setItem(MODEL_SELECTION_KEY + "." + session.id, JSON.stringify(selection));
    } catch { /* SQLite session metadata remains the durable fallback. */ }
    void api.updateSessionModel(session.id, nextProvider, nextModel).catch((reason) => {
      setError("Could not save this chat's model selection: " + String(reason));
    });
  };
  useEffect(() => {
    const selected = providers.find((provider) => provider.id === providerId);
    // Preserve the user's saved choice while its server is offline. Silently switching
    // providers here would make a restart appear to forget the selected model.
    if (!selected?.available || modelId) return;
    setModelId(selected.models[0]?.id ?? "");
  }, [providers, providerId, modelId]);

  useEffect(() => {
    if (
      !session.providerId &&
      !rememberedModel &&
      providerId === "ollama" &&
      availableProviders.length === 0 &&
      modelLibrary.length > 0
    ) {
      setProviderId("llama-cpp");
    }
  }, [availableProviders.length, modelLibrary.length, providerId, rememberedModel, session.providerId]);

  const selectManagedModel = async (path: string) => {
    if (!path || sending || loadingManagedModel) return;
    setProviderId("llama-cpp");
    setModelId(path);
    setError(null);
    setLoadingManagedModel(true);
    setStatus("Inspecting model metadata and preparing its configuration…");
    setTurnActivities([{ id: crypto.randomUUID(), label: "Model selected", detail: "The model is not loaded. It will start only when you send your first message.", state: "complete" }]);
    try {
      await onSelectManagedModel(path, (detail) => {
        setStatus(detail);
        updateActiveTurnActivity("Preparing model", detail);
      });
      rememberModel("llama-cpp", path, path);
      setStatus("Model selected. It will load into memory when you send your first message.");
      finishTurnActivity("Ready to load", "No model weights have been loaded yet.");
    } catch (reason) {
      setError("Could not inspect the selected model: " + String(reason));
      setStatus("Model preparation failed.");
      finishTurnActivity("Model preparation failed", String(reason), "error");
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
    setStatus("Restoring saved session…");
    setProviderId(session.providerId ?? rememberedModel?.providerId ?? "ollama");
    setModelId(session.modelId ?? rememberedModel?.modelId ?? "");
    void api.listSessionMessages(session.id).then((stored) => {
        if (disposed) return;
        setMessages(stored.map(({ role, content }) => ({ role, content })));
        setStatus("Session restored.");
    }).catch((reason) => {
      if (!disposed) setError("Could not restore session history: " + String(reason));
    });
    return () => { disposed = true; };
  }, [session.id]);


  useEffect(() => () => {
    const current = activeRequest.current;
    if (current) void api.cancelChat(current);
  }, []);

  const appendDelta = (delta: ChatDelta, id: string) => {
    if (delta.requestId !== id || !delta.delta) return;
    if (assistantDraft.current.length === 0) {
      updateActiveTurnActivity("Writing response", "Receiving streamed output from the selected model.");
    }
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

  const appendThinking = (event: ChatThinkingDelta, requestId: string) => {
    if (event.requestId !== requestId || !event.delta || thinkingDraft.current.length >= 256 * 1024) return;
    if (!thinkingDraft.current) {
      updateActiveTurnActivity("Reasoning", "The selected provider is streaming a separate reasoning channel.");
    }
    const remaining = 256 * 1024 - thinkingDraft.current.length;
    let end = Math.min(event.delta.length, remaining);
    while (end > 0 && !/^[\\s\\S]$/.test(event.delta[end - 1] ?? "")) end--;
    // Keep the cap byte-safe and avoid splitting a UTF-16 surrogate pair.
    if (end > 0 && end < event.delta.length) {
      const code = event.delta.charCodeAt(end - 1);
      if (code >= 0xD800 && code <= 0xDBFF) end--;
    }
    const next = event.delta.slice(0, end);
    thinkingDraft.current += next;
    setThinkingText(thinkingDraft.current);
  };

  const send = async () => {
    const text = draft.trim();
    if (!text || sending || loadingManagedModel || (!selectedProvider?.available && !managedSelectionReady) || !modelId || !session.id) return;

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
    thinkingDraft.current = "";
    setThinkingText("");
    setDraft("");
    setSending(true);
    activeRequest.current = id;
    cancelRequested.current = false;
    setError(null);
    setLastResult(null);
    setStatus("Preparing your request…");
    setTurnActivities([{ id: crypto.randomUUID(), label: "Preparing turn", detail: "Checking the selected provider and model.", state: "active" }]);

    let unlisten: (() => void) | undefined;
    let unlistenThinking: (() => void) | undefined;
    try {
      let effectiveModelId = modelId;
      if (providerId === "llama-cpp") {
        const selectedPath = selectedManagedModelPath || modelId;
        const normalizePath = (value: string | null | undefined) => value?.replace(/\\/g, "/").toLowerCase() ?? "";
        const runtimeHasSelection = Boolean(runtimeStatus?.running && normalizePath(runtimeStatus.modelPath) === normalizePath(selectedPath));
        if (!runtimeHasSelection) {
          updateActiveTurnActivity("Loading model", "Installing a compatible backend if needed, then loading model weights into memory.");
          effectiveModelId = await onLoadManagedModel(selectedPath, (detail) => {
            setStatus(detail);
            updateActiveTurnActivity("Loading model", detail);
          });
          setModelId(effectiveModelId);
          rememberModel("llama-cpp", effectiveModelId, selectedPath);
          addTurnActivity("Model ready", "The selected GGUF is loaded and the local endpoint is responding.");
        } else if (effectiveModelId === selectedPath) {
          effectiveModelId = selectedProvider?.models[0]?.id ?? effectiveModelId;
          setModelId(effectiveModelId);
        }
      }
      updateActiveTurnActivity("Thinking", "Waiting for the selected model to begin responding.");
      setStatus("Generating locally…");
      if (session.title === "New session") {
        const firstLine = text.split(/\r?\n/)[0]?.trim() ?? "New chat";
        const title = firstLine.slice(0, 64) || "New chat";
        try {
          const renamed = await api.renameSession(session.id, title);
          onSessionTitleChange(session.id, renamed.title);
        } catch (reason) {
          setError("The chat started, but its title could not be saved: " + String(reason));
        }
      }
      await api.updateSessionModel(session.id, providerId, effectiveModelId);
      await api.appendSessionMessage(session.id, "user", text);
      unlisten = await listen<ChatDelta>("chat-delta", (event) => appendDelta(event.payload, id));
      unlistenThinking = await listen<ChatThinkingDelta>("chat-thinking", (event) => appendThinking(event.payload, id));
      const result = await api.startChat({
        requestId: id,
        sessionId: session.id,
        providerId,
        model: effectiveModelId,
        messages: history,
        contextTokens,
        generation
      });
      setLastResult(result);
      if (result.content.trim()) {
        try {
          await api.appendSessionMessage(session.id, "assistant", result.content);
          assistantPersisted.current = true;
        } catch (reason) {
          setError("The response completed but could not be saved to this session: " + String(reason));
        }
      }
      setStatus("Response complete.");
      finishTurnActivity("Response complete", [
        result.outputTokens !== null ? result.outputTokens.toLocaleString() + " output tokens" : null,
        result.tokensPerSecond !== null ? result.tokensPerSecond.toFixed(1) + " tokens/s" : null,
        (result.durationMs / 1000).toFixed(1) + " seconds"
      ].filter(Boolean).join(" · "));
    } catch (reason) {
      if (!assistantPersisted.current && assistantDraft.current.trim()) {
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
        finishTurnActivity("Generation stopped", "The partial response was preserved.");
      } else {
        setError(message);
        setStatus("The provider could not complete this request.");
        finishTurnActivity("Generation failed", message, "error");
      }
    } finally {
      unlisten?.();
      unlistenThinking?.();
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
          <p>Choose a local GGUF model here. Spindle prepares its configuration now and loads it only when you send your first message.</p>
          <div><b>Spindle Runtime</b><span>Choose any model from your indexed GGUF library. Spindle prepares its configuration now and loads it only when you send your first message.</span></div>
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
              <div className="chat-message-group" key={index}>
              <article className={"chat-message " + message.role}>
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
              {message.role === "user" && index === messages.length - 2 && turnActivities.length > 0 && (
                <div className="chat-turn-timeline" aria-label="Turn activity">
                  <div className="chat-turn-heading">TURN ACTIVITY</div>
                  {turnActivities.map((activity) => (
                    <div className={"chat-turn-activity " + activity.state} key={activity.id}>
                      <span className="chat-turn-indicator" />
                      <div><strong>{activity.label}</strong><span>{activity.detail}</span></div>
                    </div>
                  ))}
                  {thinkingText && (
                    <details className="chat-thinking-card" open={sending}>
                      <summary>Provider supplied reasoning</summary>
                      <div className="chat-thinking-content">{thinkingText}</div>
                    </details>
                  )}
                </div>
              )}
              </div>
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
            disabled={(!selectedProvider?.available && !managedSelectionReady) || !modelId || sending || loadingManagedModel}
          />
          <div className="chat-composer-footer">
            <div className="chat-model-controls">
              <select
                className="chat-model-picker"
                aria-label="Model"
                title="Choose a local model"
                value={providerId === "llama-cpp"
                  ? (selectedManagedModelPath ? "llama-cpp:" + selectedManagedModelPath : "")
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
                    rememberModel(nextProvider, nextModel);
                  }
                }}
              >
                <option value="">Choose a model…</option>
                {(modelLibrary.length > 0 || selectedManagedModelPath) && (
                  <optgroup label="Spindle Runtime · GGUF">
                    {selectedManagedModelPath && !modelLibrary.some((entry) => entry.path === selectedManagedModelPath) && (
                      <option value={"llama-cpp:" + selectedManagedModelPath}>
                        {selectedManagedModelPath.split(/[\\/]/).pop() ?? selectedManagedModelPath}
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
                {selectedProvider && !selectedProvider.available && providerId !== "llama-cpp" && modelId && (
                  <optgroup label={selectedProvider.name + " · offline"}>
                    <option value={providerId + ":" + modelId}>{modelId} · unavailable</option>
                  </optgroup>
                )}
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
              {providerId === "llama-cpp" && selectedManagedModelPath && !managedRuntimeLoaded && (
                <span className="chat-model-pending">Selected · loads on first message</span>
              )}
              {providerId === "llama-cpp" && runtimeStatus?.running && (
                <button
                  type="button"
                  className="chat-refresh"
                  onClick={() => void onEjectManagedModel().catch((reason) => {
                    setError("Could not eject the managed model: " + String(reason));
                  })}
                  disabled={sending || loadingManagedModel}
                  title="Eject the managed model and release memory"
                  aria-label="Eject the managed model and release memory"
                >
                  ⏏
                </button>
              )}
            </div>
            <div className="chat-submit-controls">
              {sending ? (
                <button className="button primary stop-button" onClick={() => void stop()}>Stop</button>
              ) : (
                <button className="button primary send-button" onClick={() => void send()} disabled={!draft.trim() || (!selectedProvider?.available && !managedSelectionReady) || !modelId || loadingManagedModel}>
                  Send <span>↗</span>
                </button>
              )}
            </div>
          </div>
        </div>
        <p className="chat-footnote">This conversation is saved to {session.title === "New session" ? "the selected project" : session.title}.</p>
      </div>
    </section>
  );
}

export default ChatView;
