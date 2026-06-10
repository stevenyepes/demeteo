import React, { useState, useRef, useEffect } from "react";
import { X, Check, ChevronRight, Send } from "lucide-react";

interface SelectableOption {
  value: string;
  label: string;
  description?: string;
  current?: boolean;
}

interface CommandSelectorProps {
  title: string;
  currentLabel?: string;
  options: SelectableOption[];
  isOpen: boolean;
  freeform?: boolean;
  placeholder?: string;
  onSelect: (value: string) => void;
  onClose: () => void;
}

const CommandSelector: React.FC<CommandSelectorProps> = ({
  title,
  currentLabel,
  options,
  isOpen,
  freeform = false,
  placeholder = "Enter a value...",
  onSelect,
  onClose,
}) => {
  const [customValue, setCustomValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (isOpen) {
      setCustomValue("");
      // Focus input on open
      setTimeout(() => inputRef.current?.focus(), 100);
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const handleCustomSubmit = () => {
    const v = customValue.trim();
    if (v) {
      onSelect(v);
    }
  };

  const showList = options.length > 0;

  return (
    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4 select-none">
      <div className="bg-[#0a0a0e] border border-white/10 rounded-2xl w-full max-w-lg shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-200">
        <div className="px-6 py-4 border-b border-white/5 flex items-center justify-between bg-[#050508]">
          <h3 className="text-sm font-semibold text-white flex items-center gap-2">
            {title}
            {currentLabel && (
              <span className="text-[10px] font-mono text-cyan-400 bg-cyan-500/10 px-2 py-0.5 rounded border border-cyan-500/20">
                current: {currentLabel}
              </span>
            )}
          </h3>
          <button
            type="button"
            onClick={onClose}
            className="p-1 rounded-lg text-slate-500 hover:text-white hover:bg-white/5 transition-colors"
          >
            <X size={16} />
          </button>
        </div>

        <div className="max-h-80 overflow-y-auto p-2">
          {freeform && (
            <div className="px-2 pb-2">
              <div className="relative">
                <input
                  ref={inputRef}
                  type="text"
                  value={customValue}
                  onChange={(e) => setCustomValue(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleCustomSubmit();
                    if (e.key === "Escape") onClose();
                  }}
                  placeholder={placeholder}
                  className="w-full bg-[#050508] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 focus:shadow-[0_0_15px_rgba(6,182,212,0.15)] transition-all placeholder-slate-600"
                />
                {customValue.trim() && (
                  <button
                    type="button"
                    onClick={handleCustomSubmit}
                    className="absolute right-2 top-1/2 -translate-y-1/2 p-1.5 rounded-lg bg-cyan-500/10 text-cyan-400 hover:bg-cyan-500 hover:text-slate-900 transition-all"
                  >
                    <Send size={14} />
                  </button>
                )}
              </div>
            </div>
          )}

          {showList ? (
            <div className="flex flex-col gap-1">
              {freeform && options.length > 0 && (
                <div className="px-3 py-2 text-[10px] text-slate-600 font-mono uppercase tracking-wider">
                  Available choices
                </div>
              )}
              {options.map((opt) => (
                <button
                  key={opt.value}
                  type="button"
                  onClick={() => onSelect(opt.value)}
                  className={`flex items-center gap-3 w-full text-left px-4 py-3 rounded-xl transition-all duration-150 group cursor-pointer ${
                    opt.current
                      ? "bg-cyan-500/10 border border-cyan-500/25"
                      : "bg-transparent border border-transparent hover:bg-white/5 hover:border-white/10"
                  }`}
                >
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium text-slate-200 group-hover:text-white transition-colors">
                        {opt.label}
                      </span>
                      {opt.current && (
                        <span className="text-[10px] font-mono text-cyan-400 flex items-center gap-0.5">
                          <Check size={10} />
                          active
                        </span>
                      )}
                    </div>
                    {opt.description && (
                      <div className="text-[11px] text-slate-500 mt-0.5 line-clamp-2">
                        {opt.description}
                      </div>
                    )}
                  </div>
                  <ChevronRight
                    size={14}
                    className="text-slate-600 group-hover:text-slate-400 transition-colors flex-shrink-0"
                  />
                </button>
              ))}
            </div>
          ) : !freeform ? (
            <div className="flex flex-col items-center py-10 text-slate-500">
              <p className="text-xs">No options available</p>
              <p className="text-[10px] text-slate-600 mt-1">
                The agent did not advertise any choices
              </p>
            </div>
          ) : null}
        </div>

        <div className="px-6 py-3 border-t border-white/5 bg-[#050508] flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 rounded-lg text-xs font-medium text-slate-400 hover:text-white transition-colors"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
};

export default CommandSelector;
