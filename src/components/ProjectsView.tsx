import { useCallback, useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type {
  AgentSkill, AgentTask, AgentTaskDetail, AgentRunEvent, AgentRunSummary, LocalProviderStatus, ModelConfiguration,
  Project, ProjectMemory, ProjectSession, SessionPermissions, PermissionScope,
  GitRepositoryStatus, GitCommitSummary, TaskWorktreeDiff, SandboxStatus, SandboxExecutionResult, ValidationCommand, TaskApplicationResult
} from "../lib/types";
import "./ProjectsView.css";

interface Props {
  providers: LocalProviderStatus[];
  configuration: ModelConfiguration;
  onOpenSession: (project: Project, session: ProjectSession) => void;
}

function agentEventPresentation(item: AgentRunEvent): { title: string; detail: string; tone: string } {
  const payload = item.payload && typeof item.payload === "object"
    ? item.payload as Record<string, unknown>
    : {};
  const value = (key: string) => typeof payload[key] === "string" ? payload[key] as string : "";
  const tool = value("tool").replaceAll("_", " ");
  const target = value("target");
  const callId = value("callId");
  switch (item.kind) {
    case "tool_started":
      return { title: "Calling " + (tool || "tool"), detail: target || (callId ? "Call " + callId : "Tool execution started"), tone: "active" };
    case "tool_finished":
      return { title: (tool || "Tool") + " completed", detail: target || (payload.changed === true ? "Task worktree changed" : "Read only result"), tone: "success" };
    case "tool_failed":
      return { title: (tool || "Tool") + " failed", detail: value("error") || "The operation returned an error", tone: "error" };
    case "tool_recovered_interrupted":
      return { title: "Interrupted tool call recovered", detail: (tool || "Tool") + (callId ? " · " + callId : "") + " · outcome unknown; no automatic replay", tone: "warning" };
    case "run_started":
      return { title: "Agent started", detail: "Working in the detached task worktree", tone: "active" };
    case "run_completed":
      return { title: "Agent completed", detail: "Review the task diff before applying it", tone: "success" };
    case "run_failed":
      return { title: "Agent failed", detail: value("reason") || "The task stopped with an error", tone: "error" };
    case "run_cancelled":
      return { title: "Agent cancelled", detail: "Task worktree preserved", tone: "warning" };
    default:
      return {
        title: item.kind.replaceAll("_", " "),
        detail: value("reason") || value("command") || value("summary") || "",
        tone: "neutral"
      };
  }
}

function ProjectsView({ providers, configuration, onOpenSession }: Props) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeProject, setActiveProject] = useState<Project | null>(null);
  const [sessions, setSessions] = useState<ProjectSession[]>([]);
  const [activeSession, setActiveSession] = useState<ProjectSession | null>(null);
  const [tasks, setTasks] = useState<AgentTask[]>([]);
  const [selectedTask, setSelectedTask] = useState<AgentTaskDetail | null>(null);
  const [skills, setSkills] = useState<AgentSkill[]>([]);
  const [loadedSkill, setLoadedSkill] = useState<string | null>(null);
  const [memories, setMemories] = useState<ProjectMemory[]>([]);
  const [permissionState, setPermissionState] = useState<SessionPermissions | null>(null);
  const [gitStatus, setGitStatus] = useState<GitRepositoryStatus | null>(null);
  const [gitHistory, setGitHistory] = useState<GitCommitSummary[]>([]);
  const [worktreeDiff, setWorktreeDiff] = useState<TaskWorktreeDiff | null>(null);
  const [sandboxStatus, setSandboxStatus] = useState<SandboxStatus | null>(null);
  const [validationResult, setValidationResult] = useState<SandboxExecutionResult | null>(null);
  const [validationBusy, setValidationBusy] = useState(false);
  const [agentRunning, setAgentRunning] = useState(false);
  const [agentEvents, setAgentEvents] = useState<AgentRunEvent[]>([]);
  const [agentSummary, setAgentSummary] = useState<AgentRunSummary | null>(null);
  const [applicationStatus, setApplicationStatus] = useState<TaskApplicationResult | null>(null);
  const [memoryQuery, setMemoryQuery] = useState("");
  const [memoryKey, setMemoryKey] = useState("");
  const [memoryKind, setMemoryKind] = useState("architecture_decision");
  const [memoryContent, setMemoryContent] = useState("");
  const [projectName, setProjectName] = useState("");
  const [projectPath, setProjectPath] = useState("");
  const [taskPrompt, setTaskPrompt] = useState("");
  const [providerId, setProviderId] = useState("ollama");
  const [modelId, setModelId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState("Projects and sessions are stored in Spindle's local SQLite database.");

  const availableProviders = useMemo(() => providers.filter((item) => item.available), [providers]);
  const selectedProvider = availableProviders.find((item) => item.id === providerId) ?? availableProviders[0] ?? null;

  const refreshProjects = useCallback(async () => {
    try {
      const items = await api.listProjects();
      setProjects(items);
      setActiveProject((current) => current ? items.find((item) => item.id === current.id) ?? null : null);
    } catch (reason) {
      setError(String(reason));
    }
  }, []);

  const refreshProjectData = useCallback(async (project: Project) => {
    setError(null);
    try {
      const [sessionItems, skillItems] = await Promise.all([
        api.listSessions(project.id),
        api.listAgentSkills(project.id)
      ]);
      setSessions(sessionItems);
      setSkills(skillItems);
      setActiveSession((current) => current ? sessionItems.find((item) => item.id === current.id) ?? null : null);
    } catch (reason) {
      setError(String(reason));
    }
  }, []);

  const refreshTasks = useCallback(async (session: ProjectSession) => {
    try {
      setTasks(await api.listAgentTasks(session.id));
    } catch (reason) {
      setError(String(reason));
    }
  }, []);

  useEffect(() => { void refreshProjects(); }, [refreshProjects]);
  useEffect(() => { void api.agentSandboxStatus().then(setSandboxStatus).catch((reason) => setError(String(reason))); }, []);

  useEffect(() => {
    if (selectedProvider && selectedProvider.id !== providerId) {
      setProviderId(selectedProvider.id);
      setModelId(selectedProvider.models[0]?.id ?? "");
    } else if (selectedProvider && !selectedProvider.models.some((item) => item.id === modelId)) {
      setModelId(selectedProvider.models[0]?.id ?? "");
    }
  }, [selectedProvider, providerId, modelId]);

  const selectProject = async (project: Project) => {
    setActiveProject(project);
    setActiveSession(null);
    setSelectedTask(null);
    setTasks([]);
    setWorktreeDiff(null);
    setValidationResult(null);
    setAgentEvents([]);
    setAgentSummary(null);
    setApplicationStatus(null);
    setAgentRunning(false);
    const [git, history] = await Promise.allSettled([api.gitRepositoryStatus(project.id), api.gitRepositoryHistory(project.id, 8)]);
    setGitStatus(git.status === "fulfilled" ? git.value : null);
    setGitHistory(history.status === "fulfilled" ? history.value : []);
    await refreshProjectData(project);
    setNotice(project.name + " selected.");
  };

  const createProject = async () => {
    if (!projectPath.trim() || !projectName.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const created = await api.createProject(projectPath.trim(), projectName.trim());
      setProjectName("");
      setProjectPath("");
      await refreshProjects();
      await selectProject(created);
      setNotice("Project registered. Existing files were not changed.");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const newSession = async () => {
    if (!activeProject || busy) return;
    setBusy(true);
    try {
      const session = await api.createSession(activeProject.id);
      const updated = [session, ...sessions];
      setSessions(updated);
      setActiveSession(session);
      setPermissionState(await api.projectPermissionStatus(session.id));
      setTasks([]);
      setSelectedTask(null);
      setNotice("New independent session created.");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const selectSession = async (session: ProjectSession) => {
    setActiveSession(session);
    setSelectedTask(null);
    try { setPermissionState(await api.projectPermissionStatus(session.id)); }
    catch (reason) { setError(String(reason)); }
    await refreshTasks(session);
  };

  const startPlanning = async () => {
    if (!activeProject || !activeSession || !taskPrompt.trim() || !selectedProvider || !modelId || busy) return;
    setBusy(true);
    setError(null);
    setSelectedTask(null);
    setNotice("Inspecting the project read only and asking the selected local model for a plan…");
    try {
      const detail = await api.planProjectTask(
        activeProject.id,
        activeSession.id,
        taskPrompt.trim(),
        selectedProvider.id,
        modelId,
        configuration.runtime.contextTokens,
        configuration.generation
      );
      setTaskPrompt("");
      setSelectedTask(detail);
      await refreshTasks(activeSession);
      setNotice("Plan created. Review every step before approving it.");
    } catch (reason) {
      setError(String(reason));
      setNotice("Planning did not complete.");
      await refreshTasks(activeSession);
    } finally {
      setBusy(false);
    }
  };

  const openTask = async (task: AgentTask) => {
    setSelectedTask(null);
    setWorktreeDiff(null);
    setValidationResult(null);
    setAgentEvents([]);
    setAgentSummary(null);
    setApplicationStatus(null);
    setAgentRunning(task.state === "running");
    try {
      const [detail, events, application] = await Promise.all([
        api.agentTaskDetail(task.id),
        api.listAgentTaskEvents(task.id),
        api.agentTaskApplicationStatus(task.id)
      ]);
      setSelectedTask(detail);
      setAgentRunning(detail.task.state === "running");
      setAgentEvents(events.map((item) => ({
        taskId: item.taskId, kind: item.kind, payload: item.payload, createdAt: item.createdAt
      })));
      setApplicationStatus(application);
      try { setWorktreeDiff(await api.agentTaskWorktreeDiff(task.id)); } catch { setWorktreeDiff(null); }
    } catch (reason) { setError(String(reason)); }
  };

  const approvePlan = async () => {
    if (!selectedTask || busy) return;
    setBusy(true);
    try {
      await api.approveAgentTaskPlan(selectedTask.task.id);
      setSelectedTask(await api.agentTaskDetail(selectedTask.task.id));
      if (activeSession) await refreshTasks(activeSession);
      setNotice("Plan approved and saved. No files or commands were executed.");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const rejectPlan = async () => {
    if (!selectedTask || busy) return;
    setBusy(true);
    try {
      await api.rejectAgentTaskPlan(selectedTask.task.id);
      setSelectedTask(await api.agentTaskDetail(selectedTask.task.id));
      if (activeSession) await refreshTasks(activeSession);
      setNotice("Plan rejected.");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const grantScope = async (scope: PermissionScope) => {
    if (!activeSession || busy) return;
    setBusy(true);
    try {
      setPermissionState(await api.grantProjectPermissions(activeSession.id, [scope]));
      setNotice(scope === "process_execution" ? "Restricted validation permission granted. Arbitrary shell commands remain unavailable." : "Project write permission granted for this session.");
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };

  const createTaskWorktree = async () => {
    if (!activeProject || !activeSession || !selectedTask || busy) return;
    setBusy(true); setError(null);
    try {
      await api.createAgentTaskWorktree(activeProject.id, selectedTask.task.id);
      setWorktreeDiff(await api.agentTaskWorktreeDiff(selectedTask.task.id));
      setNotice("Isolated Git worktree created from the approved task base commit. The main working tree was not changed.");
      await refreshTasks(activeSession);
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };

  const refreshTaskWorktree = async () => {
    if (!selectedTask) return;
    try { setWorktreeDiff(await api.agentTaskWorktreeDiff(selectedTask.task.id)); }
    catch (reason) { setError(String(reason)); }
  };

  const runAgent = async () => {
    if (!selectedTask || !activeSession || !selectedProvider || !modelId || !worktreeDiff || agentRunning) return;
    setAgentRunning(true);
    setAgentSummary(null);
    setError(null);
    setNotice("Agent running in its isolated task worktree. The project working tree remains untouched.");
    try {
      const result = await api.runAgentTask({
        taskId: selectedTask.task.id,
        providerId: selectedProvider.id,
        model: modelId,
        contextTokens: configuration.runtime.contextTokens,
        generation: configuration.generation
      });
      setAgentSummary(result);
      setSelectedTask(await api.agentTaskDetail(selectedTask.task.id));
      setWorktreeDiff(await api.agentTaskWorktreeDiff(selectedTask.task.id));
      setAgentEvents((await api.listAgentTaskEvents(selectedTask.task.id)).map((item) => ({
        taskId: item.taskId, kind: item.kind, payload: item.payload, createdAt: item.createdAt
      })));
      setNotice("Agent completed its task. Review the full diff before applying it to the project.");
    } catch (reason) {
      setError(String(reason));
      try {
        setSelectedTask(await api.agentTaskDetail(selectedTask.task.id));
        setWorktreeDiff(await api.agentTaskWorktreeDiff(selectedTask.task.id));
      } catch { /* Preserve the original run error. */ }
    } finally {
      setAgentRunning(false);
      await refreshTasks(activeSession);
    }
  };

  const cancelAgent = async () => {
    if (!selectedTask || !agentRunning) return;
    try {
      const cancelled = await api.cancelAgentTask(selectedTask.task.id);
      setNotice(cancelled ? "Cancellation requested. The current model request or restricted validation worker is being stopped." : "No active run was found for this task.");
    } catch (reason) { setError(String(reason)); }
  };

  const applyAgentChanges = async () => {
    if (!selectedTask || !worktreeDiff || agentRunning || worktreeDiff.diffTruncated) return;
    setBusy(true); setError(null);
    try {
      const result = await api.applyAgentTaskChanges(selectedTask.task.id);
      setApplicationStatus(result);
      if (activeProject) setGitStatus(await api.gitRepositoryStatus(activeProject.id));
      setNotice("Reviewed task changes applied to the project working tree. Git changes are still uncommitted.");
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };

  const undoAgentChanges = async () => {
    if (!selectedTask || applicationStatus?.status !== "applied" || agentRunning) return;
    setBusy(true); setError(null);
    try {
      const result = await api.undoAgentTaskApplication(selectedTask.task.id);
      setApplicationStatus(result);
      if (activeProject) setGitStatus(await api.gitRepositoryStatus(activeProject.id));
      setNotice("The task patch was reversed. Any conflicting user edits prevented the undo.");
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };

  useEffect(() => {
    const taskId = selectedTask?.task.id;
    if (!taskId) return;
    let active = true;
    let unlisten: (() => void) | undefined;
    void listen<AgentRunEvent>("agent-run-event", ({ payload }) => {
      if (!active || payload.taskId !== taskId) return;
      setAgentEvents((current) => [...current.slice(-199), payload]);
      if (payload.kind === "run_started") {
        setAgentRunning(true);
        setSelectedTask((current) => current?.task.id === taskId
          ? { ...current, task: { ...current.task, state: "running" } } : current);
      }
      if (["run_completed", "run_failed", "run_cancelled"].includes(payload.kind)) {
        setAgentRunning(false);
        void api.agentTaskDetail(taskId).then(setSelectedTask).catch((reason) => setError(String(reason)));
        void api.agentTaskWorktreeDiff(taskId).then(setWorktreeDiff).catch(() => setWorktreeDiff(null));
        void api.agentTaskApplicationStatus(taskId).then(setApplicationStatus).catch((reason) => setError(String(reason)));
        if (activeSession) void refreshTasks(activeSession);
      }
      if (payload.kind === "tool_finished" && (payload.payload as { changed?: boolean })?.changed) {
        void api.agentTaskWorktreeDiff(taskId).then(setWorktreeDiff).catch((reason) => setError(String(reason)));
      }
    }).then((dispose) => {
      if (active) unlisten = dispose;
      else dispose();
    }).catch((reason) => setError(String(reason)));
    return () => { active = false; unlisten?.(); };
  }, [selectedTask?.task.id, activeSession, refreshTasks]);

  const runValidation = async (command: ValidationCommand) => {
    if (!selectedTask || validationBusy) return;
    setValidationBusy(true); setValidationResult(null); setError(null);
    try {
      const result = await api.runAgentTaskValidation(selectedTask.task.id, command);
      setValidationResult(result);
      setNotice(result.success ? "Restricted validation passed." : "Restricted validation completed with a nonzero exit code.");
    } catch (reason) { setError(String(reason)); }
    finally { setValidationBusy(false); }
  };

  const removeTaskWorktree = async () => {
    if (!activeProject || !selectedTask || !worktreeDiff?.status.clean || busy) return;
    setBusy(true);
    try {
      await api.removeAgentTaskWorktree(activeProject.id, selectedTask.task.id);
      setWorktreeDiff(null);
      setNotice("Clean task worktree removed. The project repository and its commits were left unchanged.");
    } catch (reason) { setError(String(reason)); }
    finally { setBusy(false); }
  };

  const saveMemory = async () => {
    if (!activeProject || !memoryKey.trim() || !memoryContent.trim() || busy) return;
    setBusy(true);
    try {
      await api.saveProjectMemory(activeProject.id, memoryKind, memoryKey.trim(), memoryContent.trim());
      setMemoryKey("");
      setMemoryContent("");
      setMemories(await api.searchProjectMemory(activeProject.id, memoryQuery.trim() || memoryKey.trim()));
      setNotice("Project memory saved. It will be treated as advisory context and checked against current files.");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const searchMemory = async () => {
    if (!activeProject || !memoryQuery.trim()) return;
    try {
      setMemories(await api.searchProjectMemory(activeProject.id, memoryQuery.trim()));
    } catch (reason) {
      setError(String(reason));
    }
  };

  const loadSkill = async (skill: AgentSkill) => {
    if (!activeProject) return;
    try {
      const result = await api.loadAgentSkill(activeProject.id, skill.name);
      setLoadedSkill(result.manifest);
      setNotice("Loaded " + skill.name + ". Skill instructions do not grant tool permissions.");
    } catch (reason) {
      setError(String(reason));
    }
  };

  return (
    <section className="projects-view">
      <div className="projects-heading">
        <div><span className="eyebrow">PROJECT WORKSPACE</span><h2>Projects & sessions</h2>
          <p>Persistent project context, independent chats, and reviewable read-only agent plans.</p></div>
        <span className="local-chip"><span /> LOCAL SQLITE</span>
      </div>

      <div className="projects-columns">
        <aside className="panel projects-rail">
          <div className="projects-section-title"><strong>Projects</strong><span>{projects.length}</span></div>
          <div className="projects-list">
            {projects.map((project) => (
              <button key={project.id} className={"project-entry " + (activeProject?.id === project.id ? "selected" : "")}
                onClick={() => void selectProject(project)}>
                <strong>{project.name}</strong><small>{project.rootPath}</small>
              </button>
            ))}
            {projects.length === 0 && <p className="projects-empty">No projects registered yet.</p>}
          </div>
          <div className="project-create">
            <label>PROJECT NAME<input value={projectName} onChange={(event) => setProjectName(event.target.value)} maxLength={160} placeholder="AnimeHub" /></label>
            <label>FOLDER PATH<input value={projectPath} onChange={(event) => setProjectPath(event.target.value)} placeholder="C:\Projects\AnimeHub" spellCheck={false} /></label>
            <button className="button primary" onClick={() => void createProject()} disabled={busy || !projectName.trim() || !projectPath.trim()}>
              {busy ? "Working…" : "Add project"}
            </button>
            <small>Spindle registers the folder. It does not copy, move, or alter project files.</small>
          </div>
        </aside>

        <div className="projects-main">
          {!activeProject ? (
            <section className="panel projects-empty-state">
              <strong>Select or add a project</strong>
              <p>Each project gets its own sessions, task plans, event history, and memory records.</p>
            </section>
          ) : (
            <>
              <section className="panel projects-sessions">
                <div className="projects-section-title"><div><strong>{activeProject.name}</strong><small>{activeProject.rootPath}</small></div>
                  <button className="button subtle" onClick={() => void newSession()} disabled={busy}>New session</button>
                </div>
                <div className="session-list">
                  {sessions.map((session) => (
                    <div className={"session-entry " + (activeSession?.id === session.id ? "selected" : "")} key={session.id}>
                      <button onClick={() => void selectSession(session)}>
                        <strong>{session.title}</strong>
                        <small>{session.providerId && session.modelId ? session.providerId + " · " + session.modelId : "No model pinned"} · {new Date(session.updatedAt * 1000).toLocaleString()}</small>
                      </button>
                      <button className="button subtle" onClick={() => onOpenSession(activeProject, session)}>Open chat</button>
                    </div>
                  ))}
                  {sessions.length === 0 && <p className="projects-empty">Create a session to begin.</p>}
                </div>
              </section>

              <section className="panel git-panel">
                <div className="projects-section-title"><div><strong>Git repository</strong><small>Read-only status/history and isolated task worktrees.</small></div>
                  {gitStatus && <span className={"task-state " + (gitStatus.clean ? "approved" : "planning")}>{gitStatus.clean ? "clean" : "modified"}</span>}</div>
                {gitStatus ? <>
                  <p className="projects-empty">{gitStatus.branch} · {gitStatus.head.slice(0, 8)} · {gitStatus.changedFiles} changed · {gitStatus.stagedFiles} staged · {gitStatus.untrackedFiles} untracked</p>
                  {gitHistory.length > 0 && <div className="task-list">{gitHistory.slice(0, 5).map((commit) => <div key={commit.commit}><span><strong>{commit.subject}</strong><small>{commit.shortCommit} · {commit.author} · {new Date(commit.committedAt * 1000).toLocaleString()}</small></span></div>)}</div>}
                </> : <p className="projects-empty">Git status is unavailable. Ensure this project folder is a Git repository and Git is on PATH.</p>}
              </section>

              {activeSession && (
                <section className="panel session-permission-panel">
                  <div className="projects-section-title"><div><strong>Session permissions</strong><small>Grants expire when Spindle exits. Audit history is stored in SQLite.</small></div>
                    <span className="planning-label">LEAST PRIVILEGE</span></div>
                  <div className="permission-scopes">
                    <span className={permissionState?.scopes.includes("project_read") ? "granted" : ""}>Project read</span>
                    <span className={permissionState?.scopes.includes("project_write") ? "granted" : ""}>Project write</span>
                    <span className={permissionState?.scopes.includes("process_execution") ? "granted" : ""}>Process execution</span>
                    <span className={permissionState?.scopes.includes("external_network") ? "granted" : ""}>External network</span>
                    <span className={permissionState?.scopes.includes("mcp_tools") ? "granted" : ""}>MCP tools</span>
                    <span className={permissionState?.scopes.includes("parallel_agents") ? "granted" : ""}>Parallel agents</span>
                  </div>
                  <div className="planner-actions">
                    {!permissionState?.scopes.includes("project_write") && <button className="button subtle" onClick={() => void grantScope("project_write")} disabled={busy}>Grant project write</button>}
                    {!permissionState?.scopes.includes("process_execution") && <button className="button subtle" onClick={() => void grantScope("process_execution")} disabled={busy || !sandboxStatus?.supported}>Allow restricted validation</button>}
                  </div>
                  <p className="projects-empty">The worker accepts only fixed Cargo format, check, and test operations. It does not expose a shell, arbitrary arguments, external network access, or model initiated process execution.</p>
                  {sandboxStatus && <small>{sandboxStatus.platform}: {sandboxStatus.isolation} · {sandboxStatus.network}{sandboxStatus.reason ? " · " + sandboxStatus.reason : ""}</small>}
                </section>
              )}
              {activeSession && (
                <section className="panel planner-panel">
                  <div className="projects-section-title"><div><strong>Task planner</strong><small>Model planning remains read-only. Approved tasks can use isolated worktrees and fixed validation commands.</small></div>
                    <span className="planning-label">PLAN MODE</span></div>
                  <div className="planner-controls">
                    <label>PROVIDER<select value={selectedProvider?.id ?? providerId} onChange={(event) => { setProviderId(event.target.value); setModelId(availableProviders.find((item) => item.id === event.target.value)?.models[0]?.id ?? ""); }}>
                      {availableProviders.map((provider) => <option key={provider.id} value={provider.id}>{provider.name}</option>)}
                    </select></label>
                    <label>MODEL<select value={modelId} onChange={(event) => setModelId(event.target.value)}>
                      {(selectedProvider?.models ?? []).map((item) => <option key={item.id} value={item.id}>{item.id}</option>)}
                    </select></label>
                  </div>
                  <textarea value={taskPrompt} onChange={(event) => setTaskPrompt(event.target.value)} rows={3} maxLength={32768}
                    placeholder="Describe the outcome, constraints, and how you want the work verified…" />
                  <div className="planner-actions">
                    <small>Uses your current model context and generation settings.</small>
                    <button className="button primary" onClick={() => void startPlanning()}
                      disabled={busy || !taskPrompt.trim() || !selectedProvider || !modelId}>
                      {busy ? "Planning…" : "Create implementation plan"}
                    </button>
                  </div>
                </section>
              )}

              {selectedTask && (
                <section className="panel plan-review">
                  <div className="projects-section-title"><div><strong>Plan review</strong><small>{selectedTask.task.title}</small></div>
                    <span className={"task-state " + selectedTask.task.state}>{selectedTask.task.state.replaceAll("_", " ")}</span></div>
                  <p className="plan-summary">{selectedTask.task.plan?.summary ?? selectedTask.task.prompt}</p>
                  <ol className="plan-steps">
                    {selectedTask.steps.map((step) => (
                      <li key={step.id}>
                        <strong>{step.title}</strong><p>{step.description}</p>
                        {step.targetPaths.length > 0 && <code>{step.targetPaths.join(" · ")}</code>}
                        {step.validation && <small>Validation: {step.validation}</small>}
                        {step.risk && <small>Risk: {step.risk}</small>}
                      </li>
                    ))}
                  </ol>
                  {selectedTask.task.state === "awaiting_approval" && (
                    <div className="plan-review-actions">
                      <span>Plan approval is separate from permission grants. Creating a worktree requires project write; validation requires restricted process permission.</span>
                      <button className="button subtle" onClick={() => void rejectPlan()} disabled={busy}>Reject</button>
                      <button className="button primary" onClick={() => void approvePlan()} disabled={busy}>Approve plan</button>
                    </div>
                  )}
                  {["approved", "running", "interrupted", "failed", "cancelled", "completed"].includes(selectedTask.task.state) && <div className="task-execution-panel">
                    <div className="projects-section-title"><div><strong>Agent execution</strong><small>Model edits are restricted to this detached task worktree. Applying changes is a separate approval.</small></div>
                      <span className={"task-state " + selectedTask.task.state}>{selectedTask.task.state.replaceAll("_", " ")}</span></div>
                    {!worktreeDiff ? (
                      <button className="button primary" onClick={() => void createTaskWorktree()}
                        disabled={busy || selectedTask.task.state !== "approved" || !permissionState?.scopes.includes("project_write") || !gitStatus?.clean}>
                        Create task worktree
                      </button>
                    ) : <>
                      <p className="projects-empty">{worktreeDiff.status.branch} · base {worktreeDiff.worktree.baseCommit.slice(0, 8)} · {worktreeDiff.status.changedFiles} changed · {worktreeDiff.status.untrackedFiles} untracked</p>
                      <p className="projects-empty">Worktree: {worktreeDiff.worktree.path}</p>
                      <p className="tool-safety-note">Native model tools run only in this detached task worktree. Changes reach the project repository only after you review and apply the final diff.</p>
                      <div className="planner-actions">
                        <button className="button subtle" onClick={() => void refreshTaskWorktree()}>Refresh diff</button>
                        {agentRunning
                          ? <button className="button subtle" onClick={() => void cancelAgent()}>Cancel agent</button>
                          : <button className="button primary" onClick={() => void runAgent()}
                              disabled={busy || !permissionState?.scopes.includes("project_write") || !selectedProvider || !modelId || !["approved", "interrupted", "failed", "cancelled"].includes(selectedTask.task.state)}>
                              {["interrupted", "failed", "cancelled"].includes(selectedTask.task.state) ? "Resume agent" : "Start agent"}
                            </button>}
                        <button className="button subtle" onClick={() => void removeTaskWorktree()}
                          disabled={busy || agentRunning || !worktreeDiff.status.clean || worktreeDiff.status.head !== worktreeDiff.worktree.baseCommit}>
                          Remove unchanged worktree
                        </button>
                      </div>
                      {worktreeDiff.diff
                        ? <pre className="skill-content">{worktreeDiff.diff}{worktreeDiff.diffTruncated ? "\n[Diff truncated at 2 MiB]" : ""}</pre>
                        : <p className="projects-empty">No file changes are present in the task worktree.</p>}
                      {selectedTask.task.state === "approved" && <div className="planner-actions">
                        <button className="button subtle" onClick={() => void runValidation("format")} disabled={validationBusy || !sandboxStatus?.supported || !permissionState?.scopes.includes("process_execution")}>Run rustfmt check</button>
                        <button className="button subtle" onClick={() => void runValidation("check")} disabled={validationBusy || !sandboxStatus?.supported || !permissionState?.scopes.includes("process_execution")}>Run cargo check</button>
                        <button className="button subtle" onClick={() => void runValidation("test")} disabled={validationBusy || !sandboxStatus?.supported || !permissionState?.scopes.includes("process_execution")}>{validationBusy ? "Running…" : "Run tests"}</button>
                      </div>}
                    </>}
                    {agentRunning && <p className="projects-empty">Agent active. Use Cancel to stop its current request or restricted validation process.</p>}
                    {agentSummary && <div className="validation-result"><strong>Agent completed · {agentSummary.turns} turns · {agentSummary.toolCalls} tool calls</strong><p>{agentSummary.finalResponse}</p></div>}
                    {applicationStatus && <p className="projects-empty">Project application: {applicationStatus.status} · {applicationStatus.patchSha256.slice(0, 12)}</p>}
                    {selectedTask.task.state === "completed" && worktreeDiff && worktreeDiff.diff && <div className="planner-actions">
                      <button className="button primary" onClick={() => void applyAgentChanges()}
                        disabled={busy || agentRunning || worktreeDiff.diffTruncated || !permissionState?.scopes.includes("project_write") || applicationStatus?.status === "applied"}>
                        Apply reviewed changes to project
                      </button>
                      {applicationStatus?.status === "applied" && <button className="button subtle" onClick={() => void undoAgentChanges()} disabled={busy || agentRunning || !permissionState?.scopes.includes("project_write")}>Undo applied changes</button>}
                    </div>}
                    {agentEvents.length > 0 && <div className="agent-event-list">
                      <strong>Run activity</strong>
                      {agentEvents.slice(-20).map((item, index) => {
                        const presentation = agentEventPresentation(item);
                        return (
                          <div className={"agent-event " + presentation.tone} key={item.taskId + item.createdAt + item.kind + index}>
                            <span>{presentation.title}</span>
                            <small>{new Date(item.createdAt * 1000).toLocaleTimeString()}{presentation.detail ? " · " + presentation.detail : ""}</small>
                          </div>
                        );
                      })}
                    </div>}
                    {validationResult && <div className="validation-result"><strong>{validationResult.command} · {validationResult.success ? "passed" : "failed"} · exit {validationResult.exitCode} · {(validationResult.durationMs / 1000).toFixed(1)}s</strong>
                      <small>{validationResult.isolation}</small>
                      {validationResult.stdout && <pre className="skill-content">{validationResult.stdout}</pre>}
                      {validationResult.stderr && <pre className="skill-content">{validationResult.stderr}</pre>}
                      {validationResult.outputTruncated && <small>Output was truncated to the configured limit.</small>}
                    </div>}
                  </div>}
                </section>
              )}

              {activeSession && tasks.length > 0 && (
                <section className="panel">
                  <div className="projects-section-title"><strong>Task history</strong><span>{tasks.length}</span></div>
                  <div className="task-list">{tasks.map((task) => (
                    <button key={task.id} onClick={() => void openTask(task)}>
                      <span><strong>{task.title}</strong><small>{new Date(task.updatedAt * 1000).toLocaleString()}</small></span>
                      <em className={"task-state " + task.state}>{task.state.replaceAll("_", " ")}</em>
                    </button>
                  ))}</div>
                </section>
              )}

              <section className="panel skill-panel">
                <div className="projects-section-title"><div><strong>Agent Skills</strong><small>Standard SKILL.md discovery, project and global locations.</small></div><span>{skills.length}</span></div>
                {skills.length === 0 ? <p className="projects-empty">No compatible skills discovered. Skills are guidance only and do not expand permissions.</p> :
                  <div className="skill-list">{skills.map((skill) => <button key={skill.name} onClick={() => void loadSkill(skill)}>
                    <strong>{skill.name}</strong><span>{skill.description}</span><small>{skill.source}</small>
                  </button>)}</div>}
                {loadedSkill && <pre className="skill-content">{loadedSkill}</pre>}
              </section>

              <section className="panel memory-panel">
                <div className="projects-section-title"><div><strong>Project memory</strong><small>Explicitly stored project facts and decisions.</small></div></div>
                <div className="memory-search"><input value={memoryQuery} onChange={(event) => setMemoryQuery(event.target.value)} placeholder="Search project memory…" />
                  <button className="button subtle" onClick={() => void searchMemory()} disabled={!memoryQuery.trim()}>Search</button></div>
                <div className="memory-create">
                  <select value={memoryKind} onChange={(event) => setMemoryKind(event.target.value)}>
                    <option value="convention">Convention</option>
                    <option value="user_preference">Preference</option>
                    <option value="architecture_decision">Architecture decision</option>
                    <option value="task_outcome">Task outcome</option>
                    <option value="failure_solution">Failure and solution</option>
                  </select>
                  <input value={memoryKey} onChange={(event) => setMemoryKey(event.target.value)} maxLength={160} placeholder="Memory key, e.g. auth-flow" />
                  <textarea value={memoryContent} onChange={(event) => setMemoryContent(event.target.value)} maxLength={8192} rows={2} placeholder="Record a concise, durable project fact or decision…" />
                  <button className="button subtle" onClick={() => void saveMemory()} disabled={busy || !memoryKey.trim() || !memoryContent.trim()}>Save memory</button>
                </div>
                {memories.map((memory) => <article key={memory.id}><strong>{memory.memoryKey}</strong><small>{memory.kind} · {memory.sourcePath || "Project scope"}</small><p>{memory.content}</p></article>)}
                {memories.length === 0 && <p className="projects-empty">No matching memory records.</p>}
              </section>
            </>
          )}
        </div>
      </div>
      <div className="projects-notice">{error ? <span className="error">{error}</span> : notice}</div>
    </section>
  );
}

export default ProjectsView;
