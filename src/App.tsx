import { useCallback, useEffect, useMemo, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { api } from "./lib/api";
import ChatView from "./components/ChatView";
import RuntimeView from "./components/RuntimeView";
import spindleLogo from "./assets/brand/spindle-logo.webp";
import {
  DEFAULT_CONFIGURATION,
  type AppInfo,
  type HardwareProfile,
  type LocalProviderStatus,
  type ModelConfiguration,
  type ModelMetadata,
  type ModelLibrary,
  type OptimizationProfile,
  type SavedPreset,
  type Project,
  type ProjectSession,
  type RuntimeBackend,
  type RuntimeStatus
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
  const [activeView, setActiveView] = useState<"settings" | "chat">("chat");
  const [activeSession, setActiveSession] = useState<ProjectSession | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [sessionsByProject, setSessionsByProject] = useState<Record<string, ProjectSession[]>>({});
  const [activeProjectId, setActiveProjectId] = useState(() => {
    try { return localStorage.getItem("spindle.active-project.v1") ?? ""; }
    catch { return ""; }
  });
  const [expandedProjectId, setExpandedProjectId] = useState("");
  const [projectsLoading, setProjectsLoading] = useState(true);
  const [sidebarBusy, setSidebarBusy] = useState(false);
  const [projectFormOpen, setProjectFormOpen] = useState(false);
  const [newProjectName, setNewProjectName] = useState("");
  const [newProjectPath, setNewProjectPath] = useState("");
  const [projectError, setProjectError] = useState<string | null>(null);
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [hardware, setHardware] = useState<HardwareProfile | null>(null);
  const [localProviders, setLocalProviders] = useState<LocalProviderStatus[]>([]);
  const [runtimeStatus, setRuntimeStatus] = useState<RuntimeStatus | null>(null);
  const [providersBusy, setProvidersBusy] = useState(false);
  const [providersError, setProvidersError] = useState<string | null>(null);
  const [modelPath, setModelPath] = useState(() => {
    try {
      const saved = localStorage.getItem("spindle.model-selection.v1");
      return saved ? (JSON.parse(saved) as { managedModelPath?: string }).managedModelPath ?? "" : "";
    } catch { return ""; }
  });
  const [modelLibrary, setModelLibrary] = useState<ModelLibrary | null>(null);
  const [modelLibraryBusy, setModelLibraryBusy] = useState(false);
  const [modelLibraryError, setModelLibraryError] = useState<string | null>(null);
  const [model, setModel] = useState<ModelMetadata | null>(null);
  const [configuration, setConfiguration] = useState<ModelConfiguration>(DEFAULT_CONFIGURATION);
  const [presets, setPresets] = useState<SavedPreset[]>([]);
  const [presetName, setPresetName] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("Detecting hardware…");
  const [optimizationWarnings, setOptimizationWarnings] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [update, setUpdate] = useState<Update | null>(null);
  const [updateState, setUpdateState] = useState("Update check has not run.");
  const [updateProgress, setUpdateProgress] = useState<number | null>(null);

  const selectModelPath = useCallback((path: string) => {
    if (path === modelPath) return;
    setModelPath(path);
    setModel(null);
    setOptimizationWarnings([]);
    setError(null);
    setMessage("New GGUF selected. Inspect it before configuring or loading the model.");
  }, [modelPath]);

  const prepareChatModel = useCallback(async (
    path: string,
    onProgress: (message: string) => void
  ): Promise<string> => {
    const candidatePath = path.trim();
    if (!candidatePath) throw new Error("Select a local GGUF model first.");

    setModelPath(candidatePath);
    setModel(null);
    setOptimizationWarnings([]);
    setError(null);

    try {
      onProgress("Inspecting GGUF metadata…");
      const inspected = await api.inspectModel(candidatePath);
      onProgress("Restoring the model's saved configuration…");
      const saved = await api.loadConfiguration(inspected.path, configuration.profile);
      let nextConfiguration: ModelConfiguration;
      let detectedHardware = hardware ?? await api.hardware();
      if (!hardware) setHardware(detectedHardware);

      if (saved) {
        nextConfiguration = { ...saved, modelPath: inspected.path };
      } else {
        onProgress("Optimizing the model for this hardware…");
        const optimized = await api.optimizeModel(inspected.path, configuration.profile);
        detectedHardware = optimized.hardware;
        nextConfiguration = { ...optimized.configuration, modelPath: inspected.path };
        setOptimizationWarnings(optimized.warnings);
      }

      setHardware(detectedHardware);
      setConfiguration(nextConfiguration);
      setModelPath(inspected.path);
      setModel(inspected);
      onProgress("Saving model configuration…");
      await api.saveConfiguration(nextConfiguration);

      onProgress("Checking the managed runtime…");
      let current = await api.runtimeStatus();
      const normalizePath = (value: string | null | undefined) =>
        value?.replace(/\\/g, "/").toLowerCase() ?? "";
      const sameModelAlreadyLoaded = current.running &&
        normalizePath(current.modelPath) === normalizePath(inspected.path);

      if (current.running && !sameModelAlreadyLoaded) {
        onProgress("Stopping the previously loaded model…");
        current = await api.stopRuntime();
        setRuntimeStatus(current);
      }

      if (!sameModelAlreadyLoaded) {
        const hasNvidia = detectedHardware?.gpus.some((gpu) =>
          gpu.vendor.toLowerCase().includes("nvidia")
        ) ?? false;
        const preferredBackends: RuntimeBackend[] = hasNvidia
          ? ["cuda", "vulkan", "cpu"]
          : ["vulkan", "cpu", "cuda"];
        const installed = new Set(
          current.installedBackends
            .filter((item) => item.installed)
            .map((item) => item.backend)
        );
        const backend = preferredBackends.find((item) => installed.has(item)) ?? preferredBackends[0];
        if (!backend) throw new Error("No compatible llama.cpp backend is available.");

        if (!installed.has(backend)) {
          onProgress("Installing the verified " + backend.toUpperCase() + " backend…");
          await api.installRuntime(backend);
          current = await api.runtimeStatus();
          setRuntimeStatus(current);
          if (!current.installedBackends.some((item) => item.backend === backend && item.installed)) {
            throw new Error("The selected llama.cpp backend did not finish installing.");
          }
        }

        onProgress("Starting " + backend.toUpperCase() + " and loading the model into memory…");
        current = await api.startRuntime(inspected.path, nextConfiguration, backend);
      }

      setRuntimeStatus(current);
      onProgress("Connecting to the loaded model…");
      const discovered = await api.localProviders();
      setLocalProviders(discovered);
      const managedProvider = discovered.find((provider) =>
        provider.id === "llama-cpp" && provider.available
      );
      const loadedModelId = managedProvider?.models[0]?.id;
      if (!loadedModelId) {
        throw new Error(
          "The model runtime started, but its local endpoint did not report a loaded model. Open Models for runtime details."
        );
      }

      return loadedModelId;
    } catch (reason) {
      setError(String(reason));
      throw reason;
    }
  }, [configuration.profile, hardware]);

  const refreshModelLibrary = useCallback(async () => {
    setModelLibraryBusy(true);
    setModelLibraryError(null);
    try {
      const library = await api.modelLibrary();
      setModelLibrary(library);
    } catch (reason) {
      setModelLibraryError(String(reason));
    } finally {
      setModelLibraryBusy(false);
    }
  }, []);

  const refreshLocalProviders = useCallback(async () => {
    setProvidersBusy(true);
    setProvidersError(null);
    try {
      const [discoveredProviders, managedRuntime] = await Promise.all([
        api.localProviders(),
        api.runtimeStatus()
      ]);
      setLocalProviders(discoveredProviders);
      setRuntimeStatus(managedRuntime);
    } catch (reason) {
      setProvidersError(String(reason));
    } finally {
      setProvidersBusy(false);
    }
  }, []);



  const ejectChatModel = useCallback(async () => {
    const stopped = await api.stopRuntime();
    setRuntimeStatus(stopped);
    setModelPath("");
    setModel(null);
    setOptimizationWarnings([]);
    await refreshLocalProviders();
  }, [refreshLocalProviders]);

  useEffect(() => {
    void refreshModelLibrary();
  }, [refreshModelLibrary]);

  useEffect(() => {
    let disposed = false;
    const restoreProjects = async () => {
      setProjectsLoading(true);
      setProjectError(null);
      try {
        const items = await api.listProjects();
        const sessionRows = await Promise.all(items.map(async (project) => ({
          projectId: project.id,
          sessions: (await api.listSessions(project.id)).sort((a, b) => b.updatedAt - a.updatedAt)
        })));
        if (disposed) return;

        const sessionMap = Object.fromEntries(sessionRows.map((item) => [item.projectId, item.sessions]));
        setProjects(items);
        setSessionsByProject(sessionMap);

        let rememberedProjectId = "";
        let rememberedSessionId = "";
        try {
          rememberedProjectId = localStorage.getItem("spindle.active-project.v1") ?? "";
          rememberedSessionId = localStorage.getItem("spindle.active-session.v1") ?? "";
        } catch { /* Local persistence is optional; SQLite remains authoritative. */ }

        const selectedProject = items.find((item) => item.id === rememberedProjectId) ?? items[0] ?? null;
        if (!selectedProject) {
          setActiveProjectId("");
          setExpandedProjectId("");
          setActiveSession(null);
          return;
        }

        setActiveProjectId(selectedProject.id);
        setExpandedProjectId(selectedProject.id);
        const availableSessions = sessionMap[selectedProject.id] ?? [];
        const selectedSession = availableSessions.find((item) => item.id === rememberedSessionId) ?? availableSessions[0] ?? null;
        setActiveSession(selectedSession);
        try {
          localStorage.setItem("spindle.active-project.v1", selectedProject.id);
          if (selectedSession) localStorage.setItem("spindle.active-session.v1", selectedSession.id);
          else localStorage.removeItem("spindle.active-session.v1");
        } catch { /* The active selection still works for this window. */ }
      } catch (reason) {
        if (!disposed) setProjectError("Could not load projects: " + String(reason));
      } finally {
        if (!disposed) setProjectsLoading(false);
      }
    };
    void restoreProjects();
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type !== "drop") return;
      const candidates = event.payload.paths.filter((path) => /\.gguf$/i.test(path));
      if (candidates.length === 0) return;
      if (candidates.length !== 1) {
        setError("Drop one GGUF model at a time.");
        setMessage("Model selection was not changed.");
        return;
      }
      const candidate = candidates[0];
      if (!candidate) return;
      setActiveView("settings");
      selectModelPath(candidate);
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    }).catch((reason: unknown) => {
      if (!disposed) setError("Could not enable GGUF file drop: " + String(reason));
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [selectModelPath]);

  useEffect(() => {
    void refreshLocalProviders();
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
  }, [refreshLocalProviders]);

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

  const inspect = async (path = modelPath) => {
    const candidatePath = path.trim();
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
      setModelPath(inspected.path);
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
    const name = presetName.trim();
    if (!model || !name || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.savePreset(name, { ...configuration, modelPath: model.path });
      setPresets(await api.listPresets());
      setPresetName("");
      setMessage("Preset saved.");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
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
    let downloadedBytes = 0;
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          downloadedBytes = 0;
          setUpdateProgress(0);
        }
        if (event.event === "Progress") {
          const data = event.data as { chunkLength?: number; contentLength?: number };
          const chunkLength = typeof data.chunkLength === "number" ? data.chunkLength : 0;
          const contentLength = typeof data.contentLength === "number" ? data.contentLength : 0;
          downloadedBytes += chunkLength;
          if (contentLength > 0) {
            setUpdateProgress(Math.min(99, (downloadedBytes / contentLength) * 100));
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

  const activeProject = projects.find((project) => project.id === activeProjectId) ?? null;

  const persistActiveSelection = (projectId: string, sessionId: string | null) => {
    try {
      localStorage.setItem("spindle.active-project.v1", projectId);
      if (sessionId) localStorage.setItem("spindle.active-session.v1", sessionId);
      else localStorage.removeItem("spindle.active-session.v1");
    } catch { /* Session persistence is still handled by the local database. */ }
  };

  const selectProject = (projectId: string) => {
    const projectSessions = sessionsByProject[projectId] ?? [];
    const nextSession = projectSessions[0] ?? null;
    setActiveProjectId(projectId);
    setExpandedProjectId(projectId);
    setActiveSession(nextSession);
    setActiveView("chat");
    persistActiveSelection(projectId, nextSession?.id ?? null);
  };

  const selectProjectSession = (projectId: string, session: ProjectSession) => {
    setActiveProjectId(projectId);
    setExpandedProjectId(projectId);
    setActiveSession(session);
    setActiveView("chat");
    persistActiveSelection(projectId, session.id);
  };

  const createChat = async (projectId = activeProjectId) => {
    if (!projectId || sidebarBusy) return;
    setSidebarBusy(true);
    setProjectError(null);
    try {
      const session = await api.createSession(projectId);
      setSessionsByProject((current) => ({
        ...current,
        [projectId]: [session, ...(current[projectId] ?? [])]
      }));
      setActiveProjectId(projectId);
      setExpandedProjectId(projectId);
      setActiveSession(session);
      setActiveView("chat");
      persistActiveSelection(projectId, session.id);
    } catch (reason) {
      setProjectError("Could not create chat: " + String(reason));
    } finally {
      setSidebarBusy(false);
    }
  };

  const createProject = async () => {
    const name = newProjectName.trim();
    const path = newProjectPath.trim();
    if (!name || !path || sidebarBusy) return;
    setSidebarBusy(true);
    setProjectError(null);
    try {
      const project = await api.createProject(path, name);
      setProjects((current) => [project, ...current]);
      setSessionsByProject((current) => ({ ...current, [project.id]: [] }));
      setNewProjectName("");
      setNewProjectPath("");
      setProjectFormOpen(false);
      setActiveProjectId(project.id);
      setExpandedProjectId(project.id);
      persistActiveSelection(project.id, null);
      const session = await api.createSession(project.id);
      setSessionsByProject((current) => ({ ...current, [project.id]: [session] }));
      setActiveSession(session);
      persistActiveSelection(project.id, session.id);
      setActiveView("chat");
    } catch (reason) {
      setProjectError("Could not create project: " + String(reason));
    } finally {
      setSidebarBusy(false);
    }
  };

  const handleSessionTitleChange = (sessionId: string, title: string) => {
    setActiveSession((current) => current?.id === sessionId ? { ...current, title } : current);
    setSessionsByProject((current) => Object.fromEntries(
      Object.entries(current).map(([projectId, sessions]) => [
        projectId,
        sessions.map((session) => session.id === sessionId ? { ...session, title } : session)
      ])
    ));
  };

  const startNewChat = () => {
    if (activeProjectId) void createChat(activeProjectId);
    else if (projects[0]) void createChat(projects[0].id);
    else setProjectFormOpen(true);
  };

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <img className="brand-logo" src={spindleLogo} alt="Spindle" />
        </div>
        <button className="sidebar-newchat" onClick={startNewChat}>
          <span aria-hidden="true">+</span>
          New chat
        </button>

        <div className="sidebar-projects-heading">
          <span>PROJECTS</span>
          <button type="button" title="Add project" aria-label="Add project" onClick={() => setProjectFormOpen((open) => !open)}>+</button>
        </div>
        {projectFormOpen && (
          <form className="sidebar-project-form" onSubmit={(event) => { event.preventDefault(); void createProject(); }}>
            <input aria-label="Project name" value={newProjectName} onChange={(event) => setNewProjectName(event.target.value)} placeholder="Project name" maxLength={100} />
            <input aria-label="Project folder path" value={newProjectPath} onChange={(event) => setNewProjectPath(event.target.value)} placeholder="Full folder path" spellCheck={false} />
            <div>
              <button type="button" className="sidebar-form-cancel" onClick={() => setProjectFormOpen(false)}>Cancel</button>
              <button type="submit" disabled={sidebarBusy || !newProjectName.trim() || !newProjectPath.trim()}>{sidebarBusy ? "Creating…" : "Create"}</button>
            </div>
          </form>
        )}
        <div className="sidebar-project-list">
          {projects.map((project) => {
            const selected = project.id === activeProjectId;
            const expanded = project.id === expandedProjectId;
            const sessions = sessionsByProject[project.id] ?? [];
            return (
              <section className={"sidebar-project-group " + (selected ? "selected" : "")} key={project.id}>
                <div className="sidebar-project-row">
                  <button type="button" className="sidebar-project-select" onClick={() => {
                    if (!selected) selectProject(project.id);
                    setExpandedProjectId(expanded ? "" : project.id);
                  }} title={project.rootPath} aria-expanded={expanded}>
                    <span className={"sidebar-project-chevron " + (expanded ? "expanded" : "")}>›</span>
                    <span className="sidebar-project-name">{project.name}</span>
                  </button>
                  <button type="button" className="sidebar-project-add" onClick={() => void createChat(project.id)} disabled={sidebarBusy} title={"New chat in " + project.name} aria-label={"New chat in " + project.name}>+</button>
                </div>
                {expanded && (
                  <div className="sidebar-chat-list">
                    {sessions.map((session) => (
                      <button
                        type="button"
                        key={session.id}
                        className={"sidebar-chat-item " + (activeSession?.id === session.id && activeView === "chat" ? "active" : "")}
                        onClick={() => selectProjectSession(project.id, session)}
                        title={session.title}
                      >
                        <span className="sidebar-chat-icon">○</span>
                        <span>{session.title || "New chat"}</span>
                      </button>
                    ))}
                    {sessions.length === 0 && <span className="sidebar-no-chats">No chats yet</span>}
                  </div>
                )}
              </section>
            );
          })}
          {!projectsLoading && projects.length === 0 && <p className="sidebar-empty-projects">Add a project to start a saved chat.</p>}
          {projectsLoading && <p className="sidebar-empty-projects">Loading projects…</p>}
          {projectError && <p className="sidebar-project-error">{projectError}</p>}
        </div>

        <div className="sidebar-bottom">
          <button className={"sidebar-settings " + (activeView === "settings" ? "active" : "")} onClick={() => setActiveView("settings")}>
            <span aria-hidden="true">⚙</span>
            Settings
          </button>
          <div className="version-line">Spindle {appInfo?.version ?? "—"}</div>
        </div>
      </aside>

      <main className="main">
        <header className="topbar">
          <div>
            <div className="eyebrow">{activeView === "chat" ? activeProject?.name ?? "PROJECT" : "SPINDLE"}</div>
            <h1>{activeView === "chat" ? activeSession?.title || "Chat" : "Settings"}</h1>
          </div>
        </header>

        <div className={"content " + (activeView === "chat" ? "content-chat" : "")}>
          {activeView === "chat" ? (
            activeSession ? (
              <ChatView
                key={activeSession.id}
                providers={localProviders}
                providersBusy={providersBusy}
                providersError={providersError}
                onRefreshProviders={() => void refreshLocalProviders()}
                modelLibrary={modelLibrary?.models ?? []}
                managedModelPath={modelPath}
                runtimeStatus={runtimeStatus}
                onSelectManagedModel={prepareChatModel}
                onEjectManagedModel={ejectChatModel}
                onOpenModelManager={() => setActiveView("settings")}
                onSessionTitleChange={handleSessionTitleChange}
                contextTokens={configuration.runtime.contextTokens}
                generation={configuration.generation}
                session={activeSession}
              />
            ) : (
              <section className="chat-start-state">
                <div className="chat-start-mark">✳</div>
                <strong>{activeProject ? activeProject.name : "Create a project to begin"}</strong>
                <p>{activeProject ? "Every conversation is saved as a session inside this project. Start a chat to continue." : "Chats are always saved inside a project. Add a project from the left sidebar to get started."}</p>
                <button className="button primary" onClick={() => void createChat(activeProjectId)} disabled={!activeProjectId || sidebarBusy}>
                  {sidebarBusy ? "Creating…" : "New chat"}
                </button>
              </section>
            )
          ) : (
            <>          <section className="notice warning">
            <strong>Project Core preview</strong>
            <span>GGUF inspection, hardware profiling, streamed local chat, provider discovery, optimization recommendations, local configuration storage, and a managed llama.cpp runtime are implemented. llama.cpp applies load-time settings and reports the values exposed by its API; some hardware and load-time settings remain unverified. Persistent projects and sessions, read-only project planning, reviewable task plans, Skill discovery, and task history are available in this preview. Agent execution, MCP tool dispatch, and the OS-isolated worker remain disabled.</span>
          </section>

          <section className="section-heading">
            <div><span className="eyebrow">01 / HARDWARE PROFILE</span><h2>Know the machine</h2></div>
            <span className="section-meta">Detected locally · never uploaded</span>
          </section>

          <div className="hardware-grid">
            <article className="metric-card">
              <div className="metric-label">PROCESSOR</div>
              <div className="metric-main">{hardware ? (hardware.cpuName ?? "CPU model name unavailable") : "Detecting…"}</div>
              <div className="metric-foot">{hardware?.logicalThreads ?? "—"} logical threads · {hardware?.physicalCores ?? "physical core count unavailable"}</div>
            </article>
            <article className="metric-card">
              <div className="metric-label">SYSTEM MEMORY</div>
              <div className="metric-main">{formatBytes(hardware?.ramTotalBytes ?? 0)}</div>
              <div className="metric-foot">{formatBytes(hardware?.ramAvailableBytes ?? 0)} available at detection</div>
            </article>
            <article className="metric-card">
              <div className="metric-label">GRAPHICS</div>
              <div className="metric-main">{hardware?.gpus[0]?.name ?? (hardware ? "No NVIDIA adapter detected" : "Detecting…")}</div>
              <div className="metric-foot">{hardware?.gpus[0] ? formatBytes(hardware.gpus[0].totalVramBytes) + " VRAM · " + (/nvidia-smi|nvml/i.test(hardware.gpus[0].detectionMethod) ? formatBytes(hardware.gpus[0].availableVramBytes) + " free" : "live VRAM telemetry unavailable") : "Checks NVIDIA driver tools and Windows device inventory"}</div>
            </article>
          </div>

          <section className="section-heading">
            <div><span className="eyebrow">02 / LOCAL PROVIDERS</span><h2>Discover model servers</h2></div>
            <button className="button subtle" onClick={() => void refreshLocalProviders()} disabled={providersBusy}>
              {providersBusy ? "Scanning…" : "Refresh providers"}
            </button>
          </section>
          <div className="provider-grid">
            {localProviders.map((provider) => (
              <article className="provider-card" key={provider.id}>
                <div className="provider-card-header">
                  <div>
                    <strong>{provider.name}</strong>
                    <small>{provider.endpoint}</small>
                  </div>
                  <span className={"provider-badge " + (provider.available ? "connected" : "offline")}>
                    {provider.available ? "CONNECTED" : "OFFLINE"}
                  </span>
                </div>
                <p>{provider.message}{provider.latencyMs !== null ? ` · ${provider.latencyMs} ms` : ""}</p>
                {provider.models.length > 0 && (
                  <ul className="provider-model-list">
                    {provider.models.slice(0, 5).map((entry) => (
                      <li key={entry.id}>
                        <span>{entry.id}</span>
                        <small>{[entry.parameterSize, entry.quantization].filter(Boolean).join(" · ")}</small>
                      </li>
                    ))}
                  </ul>
                )}
                {provider.models.length > 5 && (
                  <small className="provider-more">and {provider.models.length - 5} more model(s)</small>
                )}
              </article>
            ))}
            {localProviders.length === 0 && (
              <article className="provider-card provider-empty">
                {providersBusy ? "Checking local inference services…" : "No local provider results yet."}
              </article>
            )}
          </div>
          {providersError && <p className="provider-error">Provider discovery failed: {providersError}</p>}

          <section className="section-heading model-heading">
            <div><span className="eyebrow">03 / MODEL INSPECTION</span><h2>Inspect a GGUF model</h2></div>
            <span className="section-meta">GGUF inspection and validation</span>
          </section>

          <section className="panel model-panel">
            <div className="model-library-heading">
              <div>
                <strong>Model library</strong>
                <span>Spindle scans its library, LM Studio, GPT4All, Jan, and Hugging Face model folders. KoboldCpp's selected Download Dir can be configured. You can also drop any GGUF file from File Explorer. Files stay where they are.</span>
              </div>
              <button className="button subtle" onClick={() => void refreshModelLibrary()} disabled={modelLibraryBusy}>
                {modelLibraryBusy ? "Scanning…" : "Refresh library"}
              </button>
            </div>
            <div className="model-library-directory">
              <span>SPINDLE MODEL FOLDER</span>
              <code>{modelLibrary?.managedDirectory ?? "Creating model folder…"}</code>
            </div>
            <div className="model-library-locations">
              {(modelLibrary?.locations ?? []).map((location) => (
                <div key={location.name + location.path}>
                  <span className={location.exists ? "location-found" : "location-missing"} />
                  <strong>{location.name}</strong>
                  <code>{location.path}</code>
                  <small>{location.exists ? "Available" : "Not found"}</small>
                </div>
              ))}
            </div>
            <div className="model-library-list">
              {(modelLibrary?.models ?? []).map((entry) => (
                <button
                  type="button"
                  key={entry.path}
                  className={"model-library-entry " + (modelPath === entry.path ? "selected" : "")}
                  onClick={() => { selectModelPath(entry.path); void inspect(entry.path); }}
                  title={entry.path}
                >
                  <span>
                    <strong>{entry.fileName}</strong>
                    <small>{entry.source} · {formatBytes(entry.sizeBytes)}</small>
                  </span>
                  <code>{entry.path}</code>
                </button>
              ))}
              {modelLibrary && modelLibrary.models.length === 0 && (
                <div className="model-library-empty">No GGUF files found. Put models in the Spindle folder above or install them in LM Studio, GPT4All, or Jan, then refresh.</div>
              )}
            </div>
            {modelLibrary?.truncated && <p className="footnote">The library scan reached its safety limit. Some model files may not be listed.</p>}
            {modelLibraryError && <p className="provider-error">Model library scan failed: {modelLibraryError}</p>}
            <label className="field-label" htmlFor="model-path">MODEL FILE</label>
            <div className="path-row">
              <input id="model-path" value={modelPath} onChange={(event) => selectModelPath(event.target.value)} placeholder="Select a model above or enter a GGUF path" spellCheck={false} />
              <button className="button primary" onClick={() => void inspect()} disabled={busy}>{busy ? "Working…" : "Inspect model"}</button>
            </div>
            <p className="footnote">Drag one .gguf file from File Explorer into Spindle, or enter its full path. Spindle inspects the GGUF metadata, then llama.cpp loads the original model file for inference without copying it.</p>
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
            <div><span className="eyebrow">04 / OPTIMIZATION ENGINE</span><h2>Choose the tradeoff</h2></div>
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
            <div><span className="eyebrow">05 / INFERENCE CONFIGURATION</span><h2>Model settings</h2></div>
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
                <input
                  aria-label="Preset name"
                  className="preset-name"
                  value={presetName}
                  onChange={(event) => setPresetName(event.target.value)}
                  onKeyDown={(event) => { if (event.key === "Enter") void savePreset(); }}
                  placeholder="Preset name"
                  maxLength={80}
                  disabled={!model || busy}
                />
                <button className="button subtle" onClick={() => void savePreset()} disabled={!model || !presetName.trim() || busy}>Save preset</button>
                <button className="button primary" onClick={() => void save()} disabled={!model || busy}>Save model settings</button>
              </div>
            </div>
          </section>

          <RuntimeView
            modelPath={modelPath}
            configuration={configuration}
            hardware={hardware}
            onRuntimeChange={() => void refreshLocalProviders()}
          />

          <section className="section-heading">
            <div><span className="eyebrow">06 / APPLICATION UPDATES</span><h2>Updates</h2></div>
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
            </>
          )}
        </div>
      </main>
    </div>
  );
}

export default App;
