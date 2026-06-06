import React from "react";
import { FileCode2, Maximize2, X } from "lucide-react";

interface CodeInspectorProps {
  fileName: string;
  fileContent: string;
  onRefresh: () => void;
  onClose: () => void;
}

const CodeInspector: React.FC<CodeInspectorProps> = ({
  fileName,
  fileContent,
  onRefresh,
  onClose,
}) => {
  const codeLines = fileContent.split("\n");

  return (
    <div className="w-full h-1/2 md:w-1/2 md:h-full flex flex-col bg-[#0a0a0e] border-t md:border-t-0 md:border-l border-white/5 z-10 animate-in slide-in-from-right-8 duration-300 shadow-2xl">
      <div className="h-12 px-4 bg-[#050508] border-b border-white/5 flex items-center justify-between select-none">
        <div className="flex items-center gap-3">
          <FileCode2 size={14} className="text-cyan-500" />
          <span className="text-xs font-mono text-slate-300">{fileName}</span>
          <span className="text-[9px] text-amber-500 border border-amber-500/20 px-1.5 py-0.5 rounded bg-amber-500/10 uppercase font-bold tracking-wider">Read-only</span>
        </div>
        <div className="flex items-center gap-3 text-slate-500">
          <button type="button" className="hover:text-cyan-400 transition-colors p-1" onClick={onRefresh} title="Refresh File">
            <Maximize2 size={14} />
          </button>
          <button type="button" onClick={onClose} className="hover:text-red-400 transition-colors p-1" title="Close Inspector">
            <X size={16} />
          </button>
        </div>
      </div>
      <div className="flex-1 overflow-auto p-4 font-mono text-[13px] leading-relaxed bg-[#050508] whitespace-pre select-text">
        <table className="w-full border-collapse">
          <tbody>
            {codeLines.map((line, idx) => (
              <tr key={idx} className="hover:bg-white/5 transition-colors">
                <td className="w-12 pr-4 text-right text-slate-600 select-none border-r border-white/5 text-[10px] font-sans align-top">
                  {idx + 1}
                </td>
                <td className="pl-4 whitespace-pre text-slate-300 font-mono text-[12px] align-top">
                  {line || " "}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
};

export default CodeInspector;
