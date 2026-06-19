import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import Editor from '@monaco-editor/react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Loader2, AlertCircle, ExternalLink } from 'lucide-react';

interface ArtifactViewerProps {
  artifactPath: string | null;
  maxHeight?: string;
  /** Explicit mime type override. When set, the viewer dispatches on
   *  mime rather than file extension (e.g. `application/x-demeteo-worktree-ref`
   *  renders an "Open in editor" CTA instead of trying to display code). */
  mime?: string;
}

export const ArtifactViewer: React.FC<ArtifactViewerProps> = ({
  artifactPath,
  maxHeight = '100%',
  mime,
}) => {
  const [content, setContent] = useState<string>('');
  const [loading, setLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [viewType, setViewType] = useState<'markdown' | 'diff' | 'code' | 'worktree-ref'>('code');
  const [language, setLanguage] = useState<string>('plaintext');
  const [worktreeRef, setWorktreeRef] = useState<{ machine_id: string; branch: string; path: string } | null>(null);

  useEffect(() => {
    if (!artifactPath) {
      setContent('');
      setError(null);
      setWorktreeRef(null);
      return;
    }

    const loadArtifact = async () => {
      setLoading(true);
      setError(null);
      setWorktreeRef(null);
      try {
        const fileContent = await invoke<string>('sftp_read_file', {
          machineId: 'local',
          path: artifactPath,
        });

        // Worktree-ref dispatch via explicit mime prop
        if (mime === 'application/x-demeteo-worktree-ref') {
          try {
            const ref = JSON.parse(fileContent);
            setWorktreeRef({
              machine_id: ref.machine_id || 'local',
              branch: ref.branch || '',
              path: ref.path || '',
            });
            setViewType('worktree-ref');
            setContent(fileContent);
          } catch {
            setError('Invalid worktree reference payload');
          }
          setLoading(false);
          return;
        }

        setContent(fileContent);

        // Detect type and language
        const ext = artifactPath.split('.').pop()?.toLowerCase() || '';
        
        // Check if content looks like a unified diff
        const isUnifiedDiff = 
          ext === 'diff' || 
          ext === 'patch' ||
          fileContent.startsWith('diff --git') ||
          fileContent.includes('\n--- a/') ||
          fileContent.includes('\n+++ b/') ||
          fileContent.includes('\n@@ -');

        if (ext === 'md' || ext === 'markdown') {
          setViewType('markdown');
          setLanguage('markdown');
        } else if (isUnifiedDiff) {
          setViewType('diff');
          setLanguage('diff');
        } else {
          setViewType('code');
          // Map file extensions to Monaco editor languages
          const langMap: Record<string, string> = {
            yaml: 'yaml',
            yml: 'yaml',
            json: 'json',
            rs: 'rust',
            ts: 'typescript',
            tsx: 'typescript',
            js: 'javascript',
            jsx: 'javascript',
            py: 'python',
            sh: 'shell',
            bash: 'shell',
            toml: 'toml',
            css: 'css',
            html: 'html',
            sql: 'sql',
            go: 'go',
            cpp: 'cpp',
            c: 'c',
          };
          setLanguage(langMap[ext] || 'plaintext');
        }
      } catch (err: any) {
        console.error('Failed to read artifact:', err);
        setError(err.toString() || 'Failed to read artifact file from disk.');
      } finally {
        setLoading(false);
      }
    };

    loadArtifact();
  }, [artifactPath, mime]);

  // Loader state
  if (loading) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-12 text-slate-400 space-y-3">
        <Loader2 className="w-8 h-8 text-violet-500 animate-spin" />
        <span className="text-xs uppercase font-bold tracking-widest text-slate-500">Loading output contents...</span>
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-8 text-rose-400 text-center space-y-3">
        <AlertCircle className="w-8 h-8 text-rose-500 animate-pulse" />
        <div className="text-sm font-semibold">Failed to load step execution output</div>
        <pre className="text-xs font-mono bg-rose-950/20 border border-rose-500/20 p-4 rounded-xl max-w-md overflow-x-auto text-left leading-relaxed">
          {error}
        </pre>
      </div>
    );
  }

  // Blank state
  if (!content && viewType !== 'worktree-ref') {
    return (
      <div className="flex-1 flex items-center justify-center p-12 text-slate-500 text-center text-xs uppercase font-bold tracking-wider">
        No output content generated for this step.
      </div>
    );
  }

  // Worktree-ref CTA — navigation pointer, not content
  if (viewType === 'worktree-ref' && worktreeRef) {
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-12 text-center space-y-4">
        <ExternalLink className="w-10 h-10 text-cyan-400" />
        <div className="text-sm font-semibold text-slate-200">Worktree File Reference</div>
        <div className="text-xs text-slate-400 font-mono bg-white/5 px-3 py-1.5 rounded-lg">
          {worktreeRef.path}
        </div>
        <button
          onClick={() => {
            const url = `/editor?machine=${worktreeRef.machine_id}&branch=${worktreeRef.branch}&file=${worktreeRef.path}`;
            // Navigate to the editor view using the app's router
            window.location.hash = url;
          }}
          className="px-5 py-2 bg-cyan-600/20 border border-cyan-500/30 text-cyan-300 rounded-xl text-xs font-bold uppercase tracking-wider hover:bg-cyan-600/30 transition duration-150"
        >
          Open in Editor
        </button>
      </div>
    );
  }

  return (
    <div 
      className="flex-1 flex flex-col overflow-hidden" 
      style={{ maxHeight }}
    >
      {viewType === 'markdown' ? (
        <div className="flex-1 overflow-y-auto pr-2 space-y-1 font-sans">
          <ReactMarkdown
            remarkPlugins={[remarkGfm]}
            components={{
              // Headings
              h1: ({ children }) => (
                <h1 className="text-xl font-bold font-display text-white mt-6 mb-3 tracking-wide select-text">
                  {children}
                </h1>
              ),
              h2: ({ children }) => (
                <h2 className="text-lg font-bold font-display text-white mt-5 mb-2.5 border-b border-white/5 pb-1 tracking-wide select-text">
                  {children}
                </h2>
              ),
              h3: ({ children }) => (
                <h3 className="text-md font-bold font-display text-white mt-4 mb-2 tracking-wide select-text">
                  {children}
                </h3>
              ),
              h4: ({ children }) => (
                <h4 className="text-sm font-semibold font-display text-violet-300 mt-4 mb-2 uppercase tracking-wider select-text">
                  {children}
                </h4>
              ),
              // Paragraph & spacing
              p: ({ children }) => (
                <p className="text-sm text-slate-300 leading-relaxed my-3 select-text">
                  {children}
                </p>
              ),
              // Lists
              ul: ({ children }) => (
                <ul className="list-none space-y-2 my-4 pl-4">
                  {children}
                </ul>
              ),
              ol: ({ children }) => (
                <ol className="list-decimal space-y-2 my-4 pl-6 text-slate-300 text-sm leading-relaxed select-text">
                  {children}
                </ol>
              ),
              li: ({ children }) => (
                <li className="flex items-start gap-2.5 text-slate-300 text-sm leading-relaxed my-1.5">
                  <span className="mt-2 w-1.5 h-1.5 rounded-full bg-cyan-400 shadow-[0_0_8px_rgba(6,182,212,0.6)] shrink-0" />
                  <span className="select-text">{children}</span>
                </li>
              ),
              // Blockquote
              blockquote: ({ children }) => (
                <blockquote className="border-l-4 border-violet-500 bg-violet-950/10 backdrop-blur-md px-4 py-3 rounded-r-xl my-4 text-slate-400 text-sm leading-relaxed italic border-r border-y border-white/[0.02] select-text">
                  {children}
                </blockquote>
              ),
              // Table components
              table: ({ children }) => (
                <div className="w-full overflow-x-auto my-6 rounded-xl border border-white/5 bg-[#0d0f14]/40 backdrop-blur-md">
                  <table className="w-full border-collapse text-left text-xs font-sans">
                    {children}
                  </table>
                </div>
              ),
              thead: ({ children }) => (
                <thead className="bg-white/[0.02] border-b border-white/5">
                  {children}
                </thead>
              ),
              tbody: ({ children }) => (
                <tbody className="divide-y divide-white/[0.02]">
                  {children}
                </tbody>
              ),
              tr: ({ children }) => (
                <tr className="hover:bg-white/[0.01] transition duration-150">
                  {children}
                </tr>
              ),
              th: ({ children }) => (
                <th className="px-4 py-3 font-semibold font-display uppercase tracking-wider text-slate-200 border-r border-white/5 last:border-0">
                  {children}
                </th>
              ),
              td: ({ children }) => (
                <td className="px-4 py-3 text-slate-300 border-r border-white/5 last:border-0 leading-relaxed font-sans">
                  {children}
                </td>
              ),
              // Code highlighter (inline and block)
              code: ({ node, className, children, ...props }: any) => {
                const match = /language-(\w+)/.exec(className || '');
                const codeContent = String(children).replace(/\n$/, '');
                
                // If it is block code (starts with language or contains newlines)
                const isBlock = match || className || codeContent.includes('\n');

                if (isBlock) {
                  const lang = match ? match[1] : 'plaintext';
                  const codeHeight = Math.min(codeContent.split('\n').length * 18 + 24, 350);
                  return (
                    <div className="rounded-xl border border-white/5 overflow-hidden my-4 shadow-lg bg-[#050608]/85">
                      <div className="bg-white/[0.02] px-4 py-2 border-b border-white/5 flex justify-between items-center text-[10px] uppercase font-bold text-slate-500 tracking-wider">
                        <span>{lang}</span>
                        <button 
                          onClick={() => navigator.clipboard.writeText(codeContent)}
                          className="hover:text-white transition duration-150"
                        >
                          Copy
                        </button>
                      </div>
                      <Editor
                        height={`${codeHeight}px`}
                        language={lang}
                        theme="vs-dark"
                        value={codeContent}
                        options={{
                          readOnly: true,
                          minimap: { enabled: false },
                          scrollBeyondLastLine: false,
                          fontSize: 12,
                          lineNumbers: 'on',
                          scrollbar: {
                            vertical: 'visible',
                            horizontal: 'visible',
                            verticalScrollbarSize: 6,
                            horizontalScrollbarSize: 6,
                          },
                          hideCursorInOverviewRuler: true,
                          overviewRulerBorder: false,
                          overviewRulerLanes: 0,
                          contextmenu: false,
                          folding: false,
                          renderLineHighlight: 'none',
                          domReadOnly: true,
                        }}
                      />
                    </div>
                  );
                }

                return (
                  <code 
                    className="px-1.5 py-0.5 rounded bg-white/10 text-cyan-400 font-mono text-[11px] border border-white/5 mx-0.5 select-text font-semibold"
                    {...props}
                  >
                    {children}
                  </code>
                );
              }
            }}
          >
            {content}
          </ReactMarkdown>
        </div>
      ) : (
        <div className="flex-1 rounded-xl border border-white/5 overflow-hidden shadow-lg bg-[#050608]/85 flex flex-col">
          <div className="bg-white/[0.02] px-4 py-2 border-b border-white/5 flex justify-between items-center text-[10px] uppercase font-bold text-slate-500 tracking-wider">
            <span>{viewType === 'diff' ? 'Unified Diff' : `${language} Code`}</span>
            <button 
              onClick={() => navigator.clipboard.writeText(content)}
              className="hover:text-white transition duration-150"
            >
              Copy Complete Output
            </button>
          </div>
          <div className="flex-1">
            <Editor
              height="100%"
              language={language}
              theme="vs-dark"
              value={content}
              options={{
                readOnly: true,
                minimap: { enabled: true },
                scrollBeyondLastLine: false,
                fontSize: 12,
                lineNumbers: 'on',
                scrollbar: {
                  vertical: 'visible',
                  horizontal: 'visible',
                  verticalScrollbarSize: 8,
                  horizontalScrollbarSize: 8,
                },
                hideCursorInOverviewRuler: true,
                overviewRulerBorder: false,
                overviewRulerLanes: 0,
                contextmenu: false,
                folding: true,
                renderLineHighlight: 'all',
                domReadOnly: true,
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
};
