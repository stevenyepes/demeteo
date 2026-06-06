import React, { useState } from "react";
import { Bot, X, GitBranch, Terminal, ChevronRight } from "lucide-react";

interface NewThreadModalProps {
  isOpen: boolean;
  onClose: () => void;
  onLaunch: (title: string, mode: string, branch: string, repoPath: string) => Promise<void>;
}

const NewThreadModal: React.FC<NewThreadModalProps> = ({
  isOpen,
  onClose,
  onLaunch,
}) => {
  const [title, setTitle] = useState("");
  const [mode, setMode] = useState("worktree"); // worktree | adhoc
  const [branch, setBranch] = useState("feature/agent-oauth");
  const [repoPath, setRepoPath] = useState("/home/ubuntu/project");

  if (!isOpen) return null;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onLaunch(title, mode, branch, repoPath);
  };

  return (
    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4 select-none">
      <div className="bg-[#0a0a0e] border border-white/10 rounded-2xl w-full max-w-md shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-200">
        <div className="px-6 py-4 border-b border-white/5 flex justify-between items-center bg-[#050508]">
          <h3 className="text-sm font-semibold text-white flex items-center">
            <Bot size={16} className="mr-2 text-cyan-400" /> Initialize Agent Thread
          </h3>
          <button type="button" onClick={onClose} className="text-slate-500 hover:text-white transition-colors">
            <X size={16} />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="p-6 flex flex-col gap-5">
          <div>
            <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Thread Objective</label>
            <input
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              required
              placeholder="e.g., Fix Redis connection timeout..."
              className="w-full bg-[#050508] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50"
            />
          </div>

          <div>
            <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-2 font-semibold">Execution Sandbox Mode</label>
            <div className="grid grid-cols-2 gap-3">
              <button
                type="button"
                onClick={() => setMode("worktree")}
                className={`p-3 rounded-lg border text-left transition-all flex flex-col ${
                  mode === "worktree"
                    ? "bg-cyan-500/10 border-cyan-500/50 shadow-[0_0_15px_rgba(6,182,212,0.15)]"
                    : "bg-[#050508] border-white/5 hover:border-white/10"
                }`}
              >
                <div className={`flex items-center text-xs font-semibold mb-1 ${mode === "worktree" ? "text-cyan-400" : "text-slate-300"}`}>
                  <GitBranch size={14} className="mr-1.5" /> Project Mode
                </div>
                <div className="text-[10px] text-slate-500 leading-tight">Isolates agent in a secure Git Worktree branch.</div>
              </button>

              <button
                type="button"
                onClick={() => setMode("adhoc")}
                className={`p-3 rounded-lg border text-left transition-all flex flex-col ${
                  mode === "adhoc"
                    ? "bg-violet-500/10 border-violet-500/50 shadow-[0_0_15px_rgba(139,92,246,0.15)]"
                    : "bg-[#050508] border-white/5 hover:border-white/10"
                }`}
              >
                <div className={`flex items-center text-xs font-semibold mb-1 ${mode === "adhoc" ? "text-violet-400" : "text-slate-300"}`}>
                  <Terminal size={14} className="mr-1.5" /> Ad-Hoc Mode
                </div>
                <div className="text-[10px] text-slate-500 leading-tight">Direct directory access via Permission Proxy.</div>
              </button>
            </div>
          </div>

          {mode === "worktree" && (
            <>
              <div>
                <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Repository Location Path</label>
                <input
                  type="text"
                  value={repoPath}
                  onChange={(e) => setRepoPath(e.target.value)}
                  required
                  placeholder="/home/ubuntu/project-repo"
                  className="w-full bg-[#050508] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50"
                />
              </div>
              <div>
                <label className="block text-[10px] uppercase tracking-wider text-slate-500 mb-1.5 font-semibold">Git Branch Name</label>
                <input
                  type="text"
                  value={branch}
                  onChange={(e) => setBranch(e.target.value)}
                  required
                  placeholder="feature/fix-redis"
                  className="w-full bg-[#050508] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-slate-200 focus:outline-none focus:border-cyan-500/50"
                />
              </div>
            </>
          )}

          <div className="px-6 py-4 -mx-6 -mb-6 border-t border-white/5 bg-[#050508] flex justify-end gap-3 mt-4">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 rounded-lg text-xs font-medium text-slate-400 hover:text-white transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              className="px-5 py-2 rounded-lg text-xs font-bold bg-cyan-500 text-slate-950 hover:bg-cyan-400 transition-all flex items-center"
            >
              Launch Thread <ChevronRight size={14} className="ml-1" />
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};

export default NewThreadModal;
