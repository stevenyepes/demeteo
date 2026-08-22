import React, { memo } from 'react';
import ReactMarkdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';

interface AgentMarkdownProps {
  /** One assistant turn's prose, as the model wrote it. */
  text: string;
}

/**
 * The interviewer's prose, rendered as Markdown.
 *
 * **Memoized on the text.** A settled message's text never changes, so the
 * parse happens once per message however often the transcript re-renders
 * around it — `ArtifactViewer.rerender.test.tsx` records what it costs when a
 * markdown subtree is left to re-render with its parent. `COMPONENTS` is
 * module-scoped for the same reason: a fresh object per render defeats
 * react-markdown's own memoization of the element tree.
 *
 * **No `rehype-raw`, and no raw-HTML pass of any kind.** This is a model's
 * output. react-markdown's default of emitting embedded HTML as text is the
 * property that keeps a turn from being markup, not an oversight.
 *
 * User messages deliberately do **not** come through here — see
 * `InterviewTranscript.tsx`.
 */
function AgentMarkdownInner({ text }: AgentMarkdownProps): React.ReactElement {
  return (
    <div data-testid="agent-markdown" className="min-w-0">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={COMPONENTS}>
        {text}
      </ReactMarkdown>
    </div>
  );
}

export const AgentMarkdown = memo(AgentMarkdownInner);

/** A code block, or a wide table, scrolls inside itself; the transcript column never does. */
const SCROLLS_ITSELF = 'max-w-full overflow-x-auto';

const COMPONENTS: Components = {
  p: ({ children }) => <p className="mb-2.5 leading-relaxed last:mb-0">{children}</p>,
  h1: ({ children }) => (
    <h1 className="mt-3.5 mb-2 font-heading text-[15px] font-bold text-white first:mt-0">
      {children}
    </h1>
  ),
  h2: ({ children }) => (
    <h2 className="mt-3.5 mb-2 font-heading text-[14px] font-bold text-white first:mt-0">
      {children}
    </h2>
  ),
  h3: ({ children }) => (
    <h3 className="mt-3 mb-1.5 font-heading text-[13px] font-semibold text-slate-100 first:mt-0">
      {children}
    </h3>
  ),
  h4: ({ children }) => (
    <h4 className="mt-3 mb-1.5 font-heading text-[11px] font-semibold uppercase tracking-wider text-violet-300 first:mt-0">
      {children}
    </h4>
  ),
  ul: ({ children }) => (
    <ul className="mb-2.5 list-disc space-y-1 pl-5 marker:text-violet-400/70 last:mb-0">
      {children}
    </ul>
  ),
  ol: ({ children }) => (
    <ol className="mb-2.5 list-decimal space-y-1 pl-5 marker:text-violet-400/70 last:mb-0">
      {children}
    </ol>
  ),
  li: ({ children }) => (
    <li className="leading-relaxed [&>p]:mb-0 [&>ol]:mt-1 [&>ul]:mt-1">{children}</li>
  ),
  blockquote: ({ children }) => (
    <blockquote className="mb-2.5 rounded-r border-l-2 border-violet-400/40 bg-black/25 py-1.5 pr-2.5 pl-3 text-slate-300 italic last:mb-0">
      {children}
    </blockquote>
  ),
  hr: () => <hr className="my-3 border-white/10" />,
  strong: ({ children }) => <strong className="font-semibold text-white">{children}</strong>,
  em: ({ children }) => <em className="italic">{children}</em>,
  // `target="_blank"` is how this tree opens anything external
  // (`review/PullRequestRow.tsx`, `RemoteRunInbox.tsx`): the webview keeps the
  // app, the URL goes elsewhere. Following an agent-authored href in place
  // would replace the running app with a web page and there is no way back.
  a: ({ href, children }) => (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="text-cyan-300 underline decoration-cyan-400/40 underline-offset-2 hover:decoration-cyan-300"
    >
      {children}
    </a>
  ),
  table: ({ children }) => (
    <div className={`mb-2.5 rounded-lg border border-white/10 last:mb-0 ${SCROLLS_ITSELF}`}>
      <table className="w-full border-collapse text-left text-[12px]">{children}</table>
    </div>
  ),
  thead: ({ children }) => <thead className="bg-white/[0.03]">{children}</thead>,
  tr: ({ children }) => <tr className="border-t border-white/5 first:border-t-0">{children}</tr>,
  th: ({ children }) => (
    <th className="px-2.5 py-1.5 font-heading text-[11px] font-semibold tracking-wider text-slate-200 uppercase">
      {children}
    </th>
  ),
  td: ({ children }) => (
    <td className="px-2.5 py-1.5 align-top text-slate-300 [overflow-wrap:anywhere]">{children}</td>
  ),
  // The block wrapper below is a `<div>`, so the `<pre>` react-markdown would
  // otherwise put around it is dropped rather than nested.
  pre: ({ children }) => <>{children}</>,
  code: ({ className, children }: React.ComponentPropsWithoutRef<'code'> & { node?: unknown }) => {
    const body = String(children).replace(/\n$/, '');
    if (className || body.includes('\n')) {
      return (
        <div className={`bubble-code mb-2.5 rounded-lg last:mb-0 ${SCROLLS_ITSELF}`}>
          <code className="block px-3 py-2.5 font-mono text-[11.5px] leading-relaxed whitespace-pre text-slate-200">
            {body}
          </code>
        </div>
      );
    }
    return (
      <code className="bubble-code rounded px-1 py-0.5 font-mono text-[11.5px] text-cyan-300">
        {children}
      </code>
    );
  },
};

export default AgentMarkdown;
