import React, { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import Editor from '@monaco-editor/react';
import { DiffEditor } from '@monaco-editor/react';
import {
  ArrowLeft, FolderOpen, Folder, File, RefreshCw, GitBranch, Loader2,
  GitCommit, FilePlus, FileMinus, FileEdit,
} from 'lucide-react';

interface SftpEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number;
}

interface FileNode {
  entry: SftpEntry;
  children?: FileNode[];
  expanded: boolean;
}

interface ChangedFile {
  path: string;
  status: string; // M | A | D | R | ?
}

interface CodeEditorViewProps {
  machineId: string;
  worktreePath: string;
  branch: string;
  defaultBranch: string;
  featureTitle: string;
  initialFile?: string;
  onBack: () => void;
}

type SidebarTab = 'files' | 'changes';

const REFRESH_INTERVAL_MS = 3000;
const IGNORED = new Set(['.git', 'node_modules', 'target', '__pycache__', '.next', 'dist', 'build']);

const LANG_MAP: Record<string, string> = {
  ts: 'typescript', tsx: 'typescript', js: 'javascript', jsx: 'javascript',
  rs: 'rust', py: 'python', go: 'go', cpp: 'cpp', c: 'c', java: 'java',
  json: 'json', yaml: 'yaml', yml: 'yaml', toml: 'toml', md: 'markdown',
  markdown: 'markdown', sh: 'shell', bash: 'shell', css: 'css', html: 'html',
  sql: 'sql', xml: 'xml', tf: 'hcl', rb: 'ruby', kt: 'kotlin', swift: 'swift',
};

function langFromPath(path: string): string {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  return LANG_MAP[ext] ?? 'plaintext';
}

function sortNodes(nodes: FileNode[]): FileNode[] {
  return [...nodes].sort((a, b) => {
    if (a.entry.is_dir !== b.entry.is_dir) return a.entry.is_dir ? -1 : 1;
    return a.entry.name.localeCompare(b.entry.name);
  });
}

const STATUS_COLOR: Record<string, string> = {
  M: 'text-yellow-400',
  A: 'text-emerald-400',
  D: 'text-rose-400',
  R: 'text-blue-400',
};

const STATUS_ICON: React.FC<{ status: string }> = ({ status }) => {
  switch (status) {
    case 'A': return <FilePlus className="w-3 h-3 shrink-0" />;
    case 'D': return <FileMinus className="w-3 h-3 shrink-0" />;
    default:  return <FileEdit className="w-3 h-3 shrink-0" />;
  }
};

export const CodeEditorView: React.FC<CodeEditorViewProps> = ({
  machineId,
  worktreePath,
  branch,
  defaultBranch,
  featureTitle,
  initialFile,
  onBack,
}) => {
  const [sidebarTab, setSidebarTab] = useState<SidebarTab>('files');

  // ── File tree state ───────────────────────────────────────────────
  const [nodes, setNodes] = useState<FileNode[]>([]);
  const [treeLoading, setTreeLoading] = useState(true);
  const [treeError, setTreeError] = useState<string | null>(null);

  // ── Changes tab state ─────────────────────────────────────────────
  const [changedFiles, setChangedFiles] = useState<ChangedFile[]>([]);
  const [changesLoading, setChangesLoading] = useState(false);
  const [changesError, setChangesError] = useState<string | null>(null);

  // ── Editor state ──────────────────────────────────────────────────
  const [selectedPath, setSelectedPath] = useState<string | null>(initialFile ?? null);
  const [fileContent, setFileContent] = useState<string>('');
  const [fileLoading, setFileLoading] = useState(false);
  const [fileError, setFileError] = useState<string | null>(null);

  // ── Diff editor state (Changes tab) ──────────────────────────────
  const [diffPath, setDiffPath] = useState<string | null>(null);
  const [diffStatus, setDiffStatus] = useState<string>('M');
  const [diffOriginal, setDiffOriginal] = useState<string>('');
  const [diffModified, setDiffModified] = useState<string>('');
  const [diffLoading, setDiffLoading] = useState(false);

  const [refreshing, setRefreshing] = useState(false);
  const nodesRef = useRef<FileNode[]>([]);
  nodesRef.current = nodes;
  const selectedPathRef = useRef<string | null>(null);
  selectedPathRef.current = selectedPath;

  // ── File tree ─────────────────────────────────────────────────────
  const loadDir = useCallback(async (path: string): Promise<FileNode[]> => {
    const entries = await invoke<SftpEntry[]>('sftp_list_dir', { machineId, path });
    return sortNodes(
      entries.filter(e => !IGNORED.has(e.name)).map(e => ({ entry: e, expanded: false }))
    );
  }, [machineId]);

  const loadRoot = useCallback(async () => {
    setTreeLoading(true);
    setTreeError(null);
    try {
      setNodes(await loadDir(worktreePath));
    } catch (err) {
      setTreeError(String(err));
    } finally {
      setTreeLoading(false);
    }
  }, [loadDir, worktreePath]);

  useEffect(() => { loadRoot(); }, [loadRoot]);

  const toggleDir = useCallback(async (nodePath: string) => {
    const toggle = async (ns: FileNode[]): Promise<FileNode[]> =>
      Promise.all(ns.map(async n => {
        if (n.entry.path === nodePath) {
          if (!n.expanded && (!n.children || n.children.length === 0)) {
            return { ...n, expanded: true, children: await loadDir(nodePath) };
          }
          return { ...n, expanded: !n.expanded };
        }
        if (n.children) return { ...n, children: await toggle(n.children) };
        return n;
      }));
    setNodes(await toggle(nodesRef.current));
  }, [loadDir]);

  // ── Changed files ─────────────────────────────────────────────────
  const loadChanges = useCallback(async () => {
    setChangesLoading(true);
    setChangesError(null);
    try {
      const files = await invoke<ChangedFile[]>('git_changed_files', {
        machineId,
        worktreePath,
        baseRef: defaultBranch,
        headRef: branch,
      });
      setChangedFiles(files);
    } catch (err) {
      setChangesError(String(err));
    } finally {
      setChangesLoading(false);
    }
  }, [machineId, worktreePath, defaultBranch, branch]);

  useEffect(() => {
    if (sidebarTab === 'changes') loadChanges();
  }, [sidebarTab, loadChanges]);

  // ── Open a file for reading (Files tab) ──────────────────────────
  const openFile = useCallback(async (path: string) => {
    setDiffPath(null);
    setSelectedPath(path);
    setFileLoading(true);
    setFileError(null);
    try {
      setFileContent(await invoke<string>('sftp_read_file', { machineId, path }));
    } catch (err) {
      setFileError(String(err));
    } finally {
      setFileLoading(false);
    }
  }, [machineId]);

  useEffect(() => {
    if (initialFile) openFile(initialFile);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Open a file in diff view (Changes tab) ───────────────────────
  const openDiff = useCallback(async (file: ChangedFile) => {
    setSelectedPath(null);
    setDiffPath(file.path);
    setDiffStatus(file.status);
    setDiffLoading(true);
    try {
      const [original, modified] = await Promise.all([
        file.status === 'A'
          ? Promise.resolve('')
          : invoke<string>('git_file_at_ref', {
              machineId,
              worktreePath,
              gitRef: defaultBranch,
              filePath: file.path,
            }),
        file.status === 'D'
          ? Promise.resolve('')
          : invoke<string>('git_file_at_ref', {
              machineId,
              worktreePath,
              gitRef: branch,
              filePath: file.path,
            }),
      ]);
      setDiffOriginal(original);
      setDiffModified(modified);
    } catch (err) {
      setDiffOriginal('');
      setDiffModified(String(err));
    } finally {
      setDiffLoading(false);
    }
  }, [machineId, worktreePath, defaultBranch, branch]);

  // ── Auto-refresh current file ─────────────────────────────────────
  const refreshCurrentFile = useCallback(async () => {
    const path = selectedPathRef.current;
    if (!path) return;
    try {
      setFileContent(await invoke<string>('sftp_read_file', { machineId, path }));
    } catch { /* silent */ }
  }, [machineId]);

  useEffect(() => {
    const id = setInterval(refreshCurrentFile, REFRESH_INTERVAL_MS);
    return () => clearInterval(id);
  }, [refreshCurrentFile]);

  const handleManualRefresh = async () => {
    setRefreshing(true);
    await Promise.all([loadRoot(), loadChanges(), refreshCurrentFile()]);
    setRefreshing(false);
  };

  // ── Render ────────────────────────────────────────────────────────
  const renderTree = (ns: FileNode[], depth = 0): React.ReactNode =>
    ns.map(node => (
      <div key={node.entry.path}>
        <button
          onClick={() => node.entry.is_dir ? toggleDir(node.entry.path) : openFile(node.entry.path)}
          className={`w-full flex items-center gap-1.5 py-[3px] text-xs rounded transition-colors text-left group ${
            selectedPath === node.entry.path
              ? 'bg-violet-600/25 text-violet-200'
              : 'hover:bg-white/5 text-slate-300 hover:text-white'
          }`}
          style={{ paddingLeft: `${8 + depth * 14}px`, paddingRight: '8px' }}
          title={node.entry.path}
        >
          {node.entry.is_dir
            ? node.expanded
              ? <FolderOpen className="w-3.5 h-3.5 text-yellow-400/80 shrink-0" />
              : <Folder className="w-3.5 h-3.5 text-yellow-400/60 shrink-0" />
            : <File className="w-3.5 h-3.5 text-slate-400 shrink-0" />}
          <span className="truncate font-mono">{node.entry.name}</span>
        </button>
        {node.entry.is_dir && node.expanded && node.children && (
          <div>{renderTree(node.children, depth + 1)}</div>
        )}
      </div>
    ));

  const activeFile = diffPath ?? selectedPath;
  const activeLang = activeFile ? langFromPath(activeFile) : 'plaintext';
  const activeRelPath = activeFile ? activeFile.replace(worktreePath, '').replace(/^\//, '') : null;

  return (
    <div className="flex flex-col h-full w-full overflow-hidden bg-[#0a0c10]">
      {/* Top bar */}
      <div className="flex items-center gap-3 px-4 py-2.5 border-b border-white/5 bg-[#0d0f14]/80 shrink-0">
        <button
          onClick={onBack}
          className="flex items-center gap-1.5 text-slate-400 hover:text-white transition-colors text-xs font-medium"
        >
          <ArrowLeft className="w-4 h-4" />
          <span>{featureTitle}</span>
        </button>
        <span className="text-white/10">·</span>
        <div className="flex items-center gap-1.5 text-xs text-slate-500 font-mono">
          <GitBranch className="w-3.5 h-3.5 text-cyan-500/70" />
          <span className="text-cyan-400/80">{branch}</span>
        </div>
        <div className="flex-1" />
        <button
          onClick={handleManualRefresh}
          disabled={refreshing}
          className="flex items-center gap-1.5 text-slate-500 hover:text-white transition-colors text-xs px-2 py-1 rounded hover:bg-white/5"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${refreshing ? 'animate-spin text-cyan-400' : ''}`} />
        </button>
      </div>

      <div className="flex flex-1 overflow-hidden">
        {/* Sidebar */}
        <div className="w-56 shrink-0 border-r border-white/5 bg-[#0d0f14]/50 flex flex-col overflow-hidden">
          {/* Tab switcher */}
          <div className="flex border-b border-white/5 shrink-0">
            <button
              onClick={() => setSidebarTab('files')}
              className={`flex-1 py-2 text-[10px] uppercase font-bold tracking-widest transition-colors ${
                sidebarTab === 'files'
                  ? 'text-white border-b-2 border-violet-500 -mb-px'
                  : 'text-slate-600 hover:text-slate-400'
              }`}
            >
              Files
            </button>
            <button
              onClick={() => setSidebarTab('changes')}
              className={`flex-1 py-2 text-[10px] uppercase font-bold tracking-widest transition-colors flex items-center justify-center gap-1 ${
                sidebarTab === 'changes'
                  ? 'text-white border-b-2 border-violet-500 -mb-px'
                  : 'text-slate-600 hover:text-slate-400'
              }`}
            >
              <GitCommit className="w-3 h-3" />
              Changes
              {changedFiles.length > 0 && (
                <span className="ml-0.5 bg-violet-600/60 text-violet-200 text-[9px] font-bold rounded px-1 leading-4">
                  {changedFiles.length}
                </span>
              )}
            </button>
          </div>

          {/* Sidebar content */}
          <div className="flex-1 overflow-y-auto py-1">
            {sidebarTab === 'files' && (
              treeLoading ? (
                <div className="flex items-center justify-center py-8">
                  <Loader2 className="w-4 h-4 text-violet-400 animate-spin" />
                </div>
              ) : treeError ? (
                <div className="p-3 text-xs text-rose-400 font-mono">{treeError}</div>
              ) : renderTree(nodes)
            )}

            {sidebarTab === 'changes' && (
              changesLoading ? (
                <div className="flex items-center justify-center py-8">
                  <Loader2 className="w-4 h-4 text-violet-400 animate-spin" />
                </div>
              ) : changesError ? (
                <div className="p-3 text-xs text-rose-400 font-mono">{changesError}</div>
              ) : changedFiles.length === 0 ? (
                <div className="py-8 px-3 text-[10px] text-slate-600 uppercase tracking-widest text-center">
                  No changes vs {defaultBranch}
                </div>
              ) : changedFiles.map(f => (
                <button
                  key={f.path}
                  onClick={() => openDiff(f)}
                  className={`w-full flex items-center gap-2 px-3 py-[4px] text-xs rounded transition-colors text-left ${
                    diffPath === f.path
                      ? 'bg-violet-600/25 text-violet-200'
                      : 'hover:bg-white/5 text-slate-300 hover:text-white'
                  }`}
                  title={f.path}
                >
                  <span className={STATUS_COLOR[f.status] ?? 'text-slate-400'}>
                    <STATUS_ICON status={f.status} />
                  </span>
                  <span className="truncate font-mono flex-1">
                    {f.path.split('/').pop()}
                  </span>
                  <span className={`text-[9px] font-bold shrink-0 ${STATUS_COLOR[f.status] ?? 'text-slate-500'}`}>
                    {f.status}
                  </span>
                </button>
              ))
            )}
          </div>
        </div>

        {/* Editor pane */}
        <div className="flex-1 flex flex-col overflow-hidden">
          {/* File tab bar */}
          {activeRelPath && (
            <div className="flex items-center gap-2 px-4 py-1.5 border-b border-white/5 bg-[#0d0f14]/40 shrink-0">
              <File className="w-3.5 h-3.5 text-violet-400/70 shrink-0" />
              <span className="text-xs font-mono text-slate-300 truncate" title={activeRelPath}>
                {activeRelPath}
              </span>
              {diffPath && (
                <span className={`text-[9px] font-bold ml-1 shrink-0 ${STATUS_COLOR[diffStatus] ?? 'text-slate-500'}`}>
                  {diffStatus === 'A' ? 'added' : diffStatus === 'D' ? 'deleted' : 'modified'}
                </span>
              )}
              <span className="text-[10px] text-slate-600 ml-auto uppercase tracking-wider font-bold shrink-0">
                {diffPath ? `${defaultBranch} → ${branch}` : 'read-only'}
              </span>
            </div>
          )}

          <div className="flex-1 overflow-hidden relative">
            {/* Empty state */}
            {!selectedPath && !diffPath && (
              <div className="flex flex-col items-center justify-center h-full text-slate-600 select-none gap-2">
                <FolderOpen className="w-10 h-10 text-slate-700" />
                <span className="text-xs uppercase tracking-widest font-bold">Select a file to view</span>
              </div>
            )}

            {/* File loading */}
            {selectedPath && fileLoading && !fileContent && (
              <div className="flex items-center justify-center h-full">
                <Loader2 className="w-5 h-5 text-violet-400 animate-spin" />
              </div>
            )}

            {/* File error */}
            {selectedPath && fileError && (
              <div className="p-6 text-xs text-rose-400 font-mono">{fileError}</div>
            )}

            {/* Read-only file editor */}
            {selectedPath && !fileError && (
              <Editor
                height="100%"
                language={activeLang}
                theme="vs-dark"
                value={fileContent}
                options={{
                  readOnly: true,
                  domReadOnly: true,
                  minimap: { enabled: true },
                  scrollBeyondLastLine: false,
                  fontSize: 13,
                  lineNumbers: 'on',
                  folding: true,
                  renderLineHighlight: 'all',
                  contextmenu: false,
                  hideCursorInOverviewRuler: true,
                  overviewRulerBorder: false,
                  wordWrap: 'off',
                }}
              />
            )}

            {/* Diff loading */}
            {diffPath && diffLoading && (
              <div className="flex items-center justify-center h-full">
                <Loader2 className="w-5 h-5 text-violet-400 animate-spin" />
              </div>
            )}

            {/* Side-by-side diff editor */}
            {diffPath && !diffLoading && (
              <DiffEditor
                height="100%"
                language={activeLang}
                theme="vs-dark"
                original={diffOriginal}
                modified={diffModified}
                options={{
                  readOnly: true,
                  renderSideBySide: true,
                  minimap: { enabled: false },
                  scrollBeyondLastLine: false,
                  fontSize: 13,
                  renderOverviewRuler: false,
                  renderIndicators: true,
                  hideUnchangedRegions: { enabled: true },
                  contextmenu: false,
                }}
              />
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
