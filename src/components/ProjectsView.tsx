import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../lib/api";
import type {
  AgentSkill, AgentTask, AgentTaskDetail, LocalProviderStatus, ModelConfiguration,
  Project, ProjectMemory, ProjectSession, SessionPermissions
} from "../lib/types";
import "./ProjectsView.css";

interface Props {
  providers: LocalProviderStatus[];
  configuration: ModelConfiguration;
  onOpenSession: (project: Project, session: ProjectSession) => void;
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
    try {
      setSelectedTask(await api.agentTaskDetail(task.id));
    } catch (reason) {
      setError(String(reason));
    }
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

              {activeSession && (
                <section className="panel session-permission-panel">
                  <div className="projects-section-title"><div><strong>Session permissions</strong><small>Grants are in-memory and expire when Spindle exits.</small></div>
                    <span className="planning-label">LEAST PRIVILEGE</span></div>
                  <div className="permission-scopes">
                    <span className={permissionState?.scopes.includes("project_read") ? "granted" : ""}>Project read</span>
                    <span className={permissionState?.scopes.includes("project_write") ? "granted" : ""}>Project write</span>
                    <span className={permissionState?.scopes.includes("process_execution") ? "granted" : ""}>Process execution</span>
                    <span className={permissionState?.scopes.includes("external_network") ? "granted" : ""}>External network</span>
                    <span className={permissionState?.scopes.includes("mcp_tools") ? "granted" : ""}>MCP tools</span>
                    <span className={permissionState?.scopes.includes("parallel_agents") ? "granted" : ""}>Parallel agents</span>
                  </div>
                  <p className="projects-empty">Only project read is granted by default. Higher privilege scopes are not used by this planner and must not be enabled until the isolated execution worker is validated.</p>
                </section>
              )}
              {activeSession && (
                <section className="panel planner-panel">
                  <div className="projects-section-title"><div><strong>Read-only task planner</strong><small>Planning only. No file writes, shell, MCP, or network tools.</small></div>
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
                      <span>Approval records consent only. Execution is not enabled in this build.</span>
                      <button className="button subtle" onClick={() => void rejectPlan()} disabled={busy}>Reject</button>
                      <button className="button primary" onClick={() => void approvePlan()} disabled={busy}>Approve plan</button>
                    </div>
                  )}
                  {selectedTask.task.state === "approved" && <p className="approved-note">Approved and persisted. No files or commands have been run.</p>}
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
