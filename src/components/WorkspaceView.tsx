import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type {
  WorkspaceGrant,
  WorkspaceListing,
  WorkspaceSearchResult
} from "../lib/types";

const formatBytes = (bytes: number | null): string => {
  if (bytes === null) return "—";
  if (bytes < 1024) return bytes + " B";
  const units = ["KiB", "MiB", "GiB"];
  let value = bytes;
  let unit = -1;
  do {
    value /= 1024;
    unit++;
  } while (value >= 1024 && unit < units.length - 1);
  return value.toFixed(1) + " " + units[unit];
};

interface WorkspaceViewProps {
  onAccessChange?: () => void;
}

function WorkspaceView({ onAccessChange }: WorkspaceViewProps) {
  const [grant, setGrant] = useState<WorkspaceGrant | null>(null);
  const [rootPath, setRootPath] = useState("");
  const [allowWrite, setAllowWrite] = useState(false);
  const [listing, setListing] = useState<WorkspaceListing | null>(null);
  const [currentDirectory, setCurrentDirectory] = useState("");
  const [selectedPath, setSelectedPath] = useState("");
  const [selectedRevision, setSelectedRevision] = useState("");
  const [editorContent, setEditorContent] = useState("");
  const [dirty, setDirty] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<WorkspaceSearchResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState("No project directory is currently granted.");

  const loadDirectory = useCallback(async (session: WorkspaceGrant, path: string) => {
    setBusy(true);
    setError(null);
    try {
      const result = await api.listWorkspace(session.sessionId, path);
      setListing(result);
      setCurrentDirectory(path);
      setSelectedPath("");
      setSelectedRevision("");
      setEditorContent("");
      setDirty(false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      const active = await api.workspaceStatus();
      setGrant(active);
      if (active) {
        setRootPath(active.rootPath);
        setAllowWrite(active.canWrite);
        await loadDirectory(active, "");
        setNotice("Temporary workspace grant is active for this application session.");
      } else {
        setListing(null);
        setSelectedPath("");
        setSelectedRevision("");
        setEditorContent("");
      }
    } catch (reason) {
      setError(String(reason));
    }
  }, [loadDirectory]);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  const grantAccess = async () => {
    if (!rootPath.trim()) {
      setError("Enter the full path to a project directory.");
      return;
    }
    setBusy(true);
    setError(null);
    setNotice("Applying workspace permissions…");
    try {
      const next = await api.grantWorkspace(rootPath.trim(), allowWrite);
      setGrant(next);
      setRootPath(next.rootPath);
      setSearchResults(null);
      await loadDirectory(next, "");
      setNotice(next.canWrite
        ? "Read and write access granted until this application session ends or access is revoked."
        : "Read-only access granted until this application session ends or access is revoked.");
      onAccessChange?.();
    } catch (reason) {
      setError(String(reason));
      setNotice("Workspace access was not granted.");
    } finally {
      setBusy(false);
    }
  };

  const revokeAccess = async () => {
    if (!grant) return;
    setBusy(true);
    setError(null);
    try {
      await api.revokeWorkspace(grant.sessionId);
      setGrant(null);
      setListing(null);
      setSelectedPath("");
      setSelectedRevision("");
      setEditorContent("");
      setDirty(false);
      setSearchResults(null);
      setNotice("Workspace access revoked.");
      onAccessChange?.();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const openFile = async (relativePath: string) => {
    if (!grant || dirty) {
      if (dirty) setError("Save your edits or discard them before opening another file.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const file = await api.readWorkspaceFile(grant.sessionId, relativePath);
      setSelectedPath(file.relativePath);
      setEditorContent(file.content);
      setSelectedRevision(file.revision);
      setDirty(false);
      setNotice("Opened " + file.relativePath + " · " + formatBytes(file.sizeBytes));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const runSearch = async () => {
    if (!grant || !searchQuery.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const result = await api.searchWorkspace(grant.sessionId, searchQuery);
      setSearchResults(result);
      setNotice("Searched " + result.filesScanned.toLocaleString() + " files.");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const saveFile = async () => {
    if (!grant?.canWrite || !selectedPath || !dirty) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await api.writeWorkspaceFile(grant.sessionId, selectedPath, editorContent, selectedRevision);
      setSelectedRevision(saved.revision);
      setDirty(false);
      setNotice("Saved " + saved.relativePath + " · " + formatBytes(saved.bytesWritten));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const discardChanges = async () => {
    if (!grant || !selectedPath) {
      setDirty(false);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const file = await api.readWorkspaceFile(grant.sessionId, selectedPath);
      setEditorContent(file.content);
      setSelectedRevision(file.revision);
      setDirty(false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const navigateUp = () => {
    if (!currentDirectory || dirty) {
      if (dirty) setError("Save your edits or discard them before changing directories.");
      return;
    }
    const parent = currentDirectory.split("/").slice(0, -1).join("/");
    if (grant) void loadDirectory(grant, parent);
  };

  const breadcrumbs = currentDirectory ? currentDirectory.split("/").filter(Boolean) : [];
  const entries = listing?.entries ?? [];

  return (
    <section className="workspace-view">
      <div className="workspace-heading">
        <div>
          <span className="eyebrow">PROJECT ACCESS</span>
          <h2>Workspace</h2>
          <p>Grant a project directory to enable bounded file browsing, search, and edits.</p>
        </div>
        {grant && (
          <button className="button subtle workspace-revoke" onClick={() => void revokeAccess()} disabled={busy}>
            Revoke access
          </button>
        )}
      </div>

      <section className="panel workspace-access-panel">
        <div className="workspace-access-title">
          <div>
            <strong>{grant ? "Project directory granted" : "Grant project access"}</strong>
            <span>{grant ? grant.rootPath : "Access is not granted automatically. Choose a directory explicitly."}</span>
          </div>
          {grant && <span className="workspace-grant-state"><i /> ACTIVE</span>}
        </div>
        <div className="workspace-grant-controls">
          <label className="workspace-root-input">
            <span>PROJECT DIRECTORY</span>
            <input
              value={rootPath}
              onChange={(event) => setRootPath(event.target.value)}
              placeholder={"C:\\Users\\You\\Documents\\MyProject"}
              spellCheck={false}
              disabled={busy}
            />
          </label>
          <label className="workspace-write-toggle">
            <input type="checkbox" checked={allowWrite} onChange={(event) => setAllowWrite(event.target.checked)} disabled={busy} />
            <span>Allow file writes</span>
          </label>
          <button className="button primary" onClick={() => void grantAccess()} disabled={busy || !rootPath.trim()}>
            {busy ? "Working…" : grant ? "Update permissions" : "Grant access"}
          </button>
        </div>
        <div className="workspace-permission-summary">
          <span><b>READ</b> {grant?.canRead ? "Granted" : "Not granted"}</span>
          <span><b>WRITE</b> {grant?.canWrite ? "Granted" : "Not granted"}</span>
          <span><b>LIFETIME</b> Current app session only</span>
        </div>
        <p className="footnote">Sensitive files such as .env files, SSH keys, cloud credentials, and private certificates are excluded. File writes require a separate grant. No terminal or command execution is enabled.</p>
      </section>

      {error && <div className="chat-error" role="alert">{error}</div>}

      {grant ? (
        <div className="workspace-layout">
          <section className="panel workspace-browser">
            <div className="workspace-pane-heading">
              <div><h3>Project files</h3><span>{listing?.entries.length ?? 0} entries</span></div>
              <button className="button subtle" onClick={() => void loadDirectory(grant, currentDirectory)} disabled={busy || dirty}>Refresh</button>
            </div>
            <div className="workspace-breadcrumbs">
              <button onClick={() => void loadDirectory(grant, "")} disabled={busy || dirty}>ROOT</button>
              {breadcrumbs.map((part, index) => {
                const path = breadcrumbs.slice(0, index + 1).join("/");
                return <span key={path}><i>/</i><button onClick={() => void loadDirectory(grant, path)} disabled={busy || dirty}>{part}</button></span>;
              })}
            </div>
            {currentDirectory && (
              <button className="workspace-parent" onClick={navigateUp} disabled={busy || dirty}>↑ Parent directory</button>
            )}
            <div className="workspace-entry-list">
              {entries.map((entry) => (
                <button
                  key={entry.relativePath}
                  className={"workspace-entry " + (selectedPath === entry.relativePath ? "selected" : "")}
                  onClick={() => entry.kind === "directory"
                    ? void loadDirectory(grant, entry.relativePath)
                    : void openFile(entry.relativePath)}
                  disabled={busy || dirty}
                >
                  <span className={"workspace-entry-icon " + entry.kind}>{entry.kind === "directory" ? "▸" : "·"}</span>
                  <span className="workspace-entry-name">{entry.name}</span>
                  <small>{entry.kind === "file" ? formatBytes(entry.sizeBytes) : "Folder"}</small>
                </button>
              ))}
              {entries.length === 0 && <div className="workspace-empty">This directory is empty.</div>}
            </div>
            {listing?.truncated && <p className="footnote">Directory listing capped at 500 entries.</p>}
          </section>

          <section className="panel workspace-editor-panel">
            <div className="workspace-pane-heading">
              <div><h3>{selectedPath || "File editor"}</h3><span>{selectedPath ? formatBytes(new TextEncoder().encode(editorContent).byteLength) : "Select a text file to inspect it"}</span></div>
              {selectedPath && <span className={"workspace-dirty " + (dirty ? "unsaved" : "")}>{dirty ? "UNSAVED" : "SAVED"}</span>}
            </div>
            {selectedPath ? (
              <>
                <textarea
                  className="workspace-editor"
                  value={editorContent}
                  onChange={(event) => { setEditorContent(event.target.value); setDirty(true); }}
                  spellCheck={false}
                  disabled={!grant.canWrite || busy}
                  aria-label={"Editor for " + selectedPath}
                />
                <div className="workspace-editor-actions">
                  <span>{grant.canWrite ? "Changes are written only after Save." : "Read-only grant. Regrant with file writes enabled to edit."}</span>
                  <div>
                    {dirty && <button className="button subtle" onClick={() => void discardChanges()} disabled={busy}>Discard</button>}
                    <button className="button primary" onClick={() => void saveFile()} disabled={!grant.canWrite || !dirty || busy}>Save file</button>
                  </div>
                </div>
              </>
            ) : (
              <div className="workspace-empty workspace-editor-empty">
                <strong>No file selected</strong>
                <p>Choose a text file from the project tree or search for a symbol below.</p>
              </div>
            )}
          </section>
        </div>
      ) : (
        <section className="panel workspace-empty workspace-ungranted">
          <strong>Workspace access is closed</strong>
          <p>Grant a directory above. The application will hold the permission in memory only, and access ends when it is revoked or Spindle closes.</p>
        </section>
      )}

      {grant && (
        <section className="panel workspace-search-panel">
          <div className="workspace-pane-heading">
            <div><h3>Search project text</h3><span>Literal, case-insensitive search across bounded text files</span></div>
          </div>
          <div className="workspace-search-controls">
            <input value={searchQuery} onChange={(event) => setSearchQuery(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void runSearch(); }} placeholder="Search for a function, class, or phrase…" maxLength={256} disabled={busy} />
            <button className="button subtle" onClick={() => void runSearch()} disabled={busy || !searchQuery.trim()}>Search</button>
          </div>
          {searchResults && (
            <div className="workspace-search-results">
              {searchResults.matches.map((match, index) => (
                <button key={match.relativePath + ":" + match.lineNumber + ":" + index} onClick={() => void openFile(match.relativePath)} disabled={busy || dirty}>
                  <strong>{match.relativePath}:{match.lineNumber}</strong>
                  <code>{match.preview}</code>
                </button>
              ))}
              {searchResults.matches.length === 0 && <p className="workspace-empty">No matches found.</p>}
              <small>Scanned {searchResults.filesScanned.toLocaleString()} files · {formatBytes(searchResults.bytesScanned)}{searchResults.truncated ? " · Results capped" : ""}</small>
            </div>
          )}
        </section>
      )}

      <div className="workspace-status-line"><span className={"status-dot " + (error ? "red" : "")} />{error ?? notice}</div>
    </section>
  );
}

export default WorkspaceView;
