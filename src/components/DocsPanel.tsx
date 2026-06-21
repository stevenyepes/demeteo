import React, { useState, useEffect } from 'react';
import { X, BookOpen, ChevronRight, FileText } from 'lucide-react';

interface DocPage {
  slug: string;
  title: string;
}

const DOC_INDEX: DocPage[] = [
  { slug: 'first-project', title: 'Your First Project' },
  { slug: 'how-workflows-work', title: 'How Workflows Work' },
  { slug: 'connecting-providers', title: 'Connecting Providers' },
  { slug: 'feature-branch-model', title: 'Feature Branch Model' },
  { slug: 'conflict-resolution', title: 'Conflict Resolution' },
  { slug: 'troubleshooting', title: 'Troubleshooting' },
  { slug: 'keyboard-shortcuts', title: 'Keyboard Shortcuts' },
];

interface DocsPanelProps {
  isOpen: boolean;
  onClose: () => void;
}

const DocsPanel: React.FC<DocsPanelProps> = ({ isOpen, onClose }) => {
  const [selectedSlug, setSelectedSlug] = useState<string>('first-project');
  const [content, setContent] = useState<string>('');
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!isOpen) return;
    loadDoc(selectedSlug);
  }, [isOpen, selectedSlug]);

  const loadDoc = async (slug: string) => {
    setLoading(true);
    try {
      const resp = await fetch(`/src/docs/${slug}.md`);
      const text = await resp.text();
      setContent(text);
    } catch {
      // Fallback: read from public path
      try {
        const resp = await fetch(`/docs/${slug}.md`);
        const text = await resp.text();
        setContent(text);
      } catch {
        setContent(`# ${slug}\n\nDocument not found.`);
      }
    } finally {
      setLoading(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center" onClick={onClose}>
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" />
      <div
        className="relative w-full max-w-3xl h-[80vh] glass-panel border border-white/10 rounded-xl shadow-2xl overflow-hidden flex flex-col"
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-white/5 bg-[#0d0f14]/80 shrink-0">
          <div className="flex items-center gap-2">
            <BookOpen className="w-5 h-5 text-cyan-400" />
            <h2 className="text-sm font-bold font-outfit text-white">Documentation</h2>
          </div>
          <button
            onClick={onClose}
            className="p-1 text-slate-400 hover:text-white rounded hover:bg-white/5 transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        <div className="flex flex-1 overflow-hidden">
          {/* Sidebar nav */}
          <nav className="w-56 border-r border-white/5 bg-[#08090c]/40 overflow-y-auto shrink-0 p-2 space-y-0.5">
            {DOC_INDEX.map(page => (
              <button
                key={page.slug}
                onClick={() => setSelectedSlug(page.slug)}
                className={`w-full text-left flex items-center gap-2 px-3 py-2 rounded-lg text-xs transition-all ${
                  selectedSlug === page.slug
                    ? 'bg-cyan-500/10 text-cyan-300 border border-cyan-500/20'
                    : 'text-slate-400 hover:text-white hover:bg-white/5'
                }`}
              >
                <FileText className="w-3.5 h-3.5 shrink-0" />
                <span className="truncate">{page.title}</span>
                {selectedSlug === page.slug && (
                  <ChevronRight className="w-3 h-3 ml-auto shrink-0 text-cyan-400" />
                )}
              </button>
            ))}
          </nav>

          {/* Content */}
          <div className="flex-1 overflow-y-auto p-6 bg-[#08090c]/60">
            {loading ? (
              <div className="flex items-center justify-center h-full">
                <div className="w-6 h-6 border-2 border-cyan-400 border-t-transparent rounded-full animate-spin" />
              </div>
            ) : (
              <div className="prose prose-invert prose-sm max-w-none">
                <SimpleMarkdown text={content} />
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};

/** Minimal markdown renderer for docs without pulling in react-markdown
 *  as a runtime dep. Supports headings, paragraphs, code blocks, inline
 *  code, lists, bold, and horizontal rules — enough for our 7 docs. */
function SimpleMarkdown({ text }: { text: string }) {
  const lines = text.split('\n');
  const elements: React.ReactNode[] = [];
  let inCodeBlock = false;
  let codeBuffer: string[] = [];

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    if (line.startsWith('```')) {
      if (inCodeBlock) {
        elements.push(
          <pre key={`pre-${i}`} className="bg-black/40 border border-white/10 rounded-lg p-4 overflow-x-auto my-3">
            <code className="text-xs font-mono text-slate-200 whitespace-pre-wrap">{codeBuffer.join('\n')}</code>
          </pre>
        );
        codeBuffer = [];
        inCodeBlock = false;
      } else {
        inCodeBlock = true;
      }
      continue;
    }

    if (inCodeBlock) {
      codeBuffer.push(line);
      continue;
    }

    if (line.startsWith('# ')) {
      elements.push(
        <h1 key={i} className="text-xl font-bold font-outfit text-white mb-4 mt-2">{line.slice(2)}</h1>
      );
    } else if (line.startsWith('## ')) {
      elements.push(
        <h2 key={i} className="text-base font-bold font-outfit text-white mb-3 mt-5">{line.slice(3)}</h2>
      );
    } else if (line.startsWith('### ')) {
      elements.push(
        <h3 key={i} className="text-sm font-bold font-outfit text-white mb-2 mt-4">{line.slice(4)}</h3>
      );
    } else if (/^- /.test(line)) {
      elements.push(
        <li key={i} className="text-sm text-slate-300 ml-4 mb-1 list-disc">{line.slice(2)}</li>
      );
    } else if (/^\d+\. /.test(line)) {
      elements.push(
        <li key={i} className="text-sm text-slate-300 ml-4 mb-1 list-decimal">{line.replace(/^\d+\. /, '')}</li>
      );
    } else if (/^\|---/.test(line) || /^\|/.test(line)) {
      // skip table formatting for now
    } else if (line.trim() === '---') {
      elements.push(<hr key={i} className="border-white/10 my-4" />);
    } else if (line.trim() === '') {
      // empty lines between blocks
    } else {
      elements.push(
        <p key={i} className="text-sm text-slate-300 mb-3 leading-relaxed">
          <InlineMarkdown text={line} />
        </p>
      );
    }
  }

  return <div>{elements}</div>;
}

function InlineMarkdown({ text }: { text: string }) {
  // Bold: **text**
  const parts = text.split(/(\*\*[^*]+\*\*)/g);
  const nodes = parts.map((part, idx) => {
    if (part.startsWith('**') && part.endsWith('**')) {
      return <strong key={idx} className="text-white font-semibold">{part.slice(2, -2)}</strong>;
    }
    // Inline code: `text`
    const codeParts = part.split(/(`[^`]+`)/g);
    return codeParts.map((cp, cidx) => {
      if (cp.startsWith('`') && cp.endsWith('`')) {
        return (
          <code key={`${idx}-${cidx}`} className="px-1.5 py-0.5 bg-black/40 border border-white/10 rounded text-[11px] font-mono text-cyan-300">
            {cp.slice(1, -1)}
          </code>
        );
      }
      return <span key={`${idx}-${cidx}`}>{cp}</span>;
    });
  });
  return <>{nodes}</>;
}

export default DocsPanel;
