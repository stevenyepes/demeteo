/**
 * Artifact classification shared by every run surface — the step inspector
 * (`NodePanel`, via `lib/stepArtifacts.ts`) and the canvas — so a file cannot
 * read as one kind here and another there.
 *
 * One quirk is load-bearing and deliberate: `.worktree-ref.json` is matched
 * *after* plain `.json`, so today it classifies as `json`. Surfaces render
 * off that verdict, so reordering the two is a UI change, not a cleanup.
 */
import {
  FileText,
  FileCode,
  FileJson,
  FileQuestion,
  GitMerge,
} from 'lucide-react';

export type ArtifactKind =
  | 'markdown'
  | 'diff'
  | 'json'
  | 'code'
  | 'text'
  | 'worktree-ref'
  | 'unknown';

export const ARTIFACT_KIND_LABELS: Record<ArtifactKind, string> = {
  markdown: 'Markdown',
  diff: 'Code Diff',
  json: 'JSON',
  code: 'Code',
  text: 'Text',
  'worktree-ref': 'File Reference',
  unknown: 'File',
};

export const ARTIFACT_KIND_COLORS: Record<ArtifactKind, string> = {
  markdown: 'text-cyan-400',
  diff: 'text-violet-400',
  json: 'text-amber-400',
  code: 'text-emerald-400',
  text: 'text-slate-400',
  'worktree-ref': 'text-cyan-400',
  unknown: 'text-slate-500',
};

const CODE_EXTS = [
  'ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'py', 'rb', 'rs', 'go', 'java',
  'kt', 'kts', 'swift', 'c', 'h', 'cpp', 'cc', 'cxx', 'hpp', 'hxx', 'sh',
  'bash', 'zsh', 'yaml', 'yml', 'toml', 'sql', 'vue', 'svelte', 'css',
  'html', 'htm', 'xml',
];

export function classifyArtifact(
  path: string,
): { kind: ArtifactKind; ext: string; basename: string } {
  const lower = path.toLowerCase();
  const filename = path.split('/').pop() || path;
  if (lower.endsWith('.diff') || lower.endsWith('.patch')) {
    return { kind: 'diff', ext: filename.split('.').pop() || 'diff', basename: filename };
  }
  if (lower.endsWith('.md') || lower.endsWith('.markdown')) {
    return { kind: 'markdown', ext: 'md', basename: filename };
  }
  if (lower.endsWith('.json')) {
    return { kind: 'json', ext: 'json', basename: filename };
  }
  if (lower.endsWith('.worktree-ref.json')) {
    return { kind: 'worktree-ref', ext: 'json', basename: filename };
  }
  const ext = filename.includes('.') ? filename.split('.').pop()!.toLowerCase() : '';
  if (CODE_EXTS.includes(ext)) {
    return { kind: 'code', ext, basename: filename };
  }
  if (ext === 'txt' || ext === 'csv' || !ext) {
    return { kind: 'text', ext: ext || 'txt', basename: filename };
  }
  return { kind: 'unknown', ext, basename: filename };
}

export function ArtifactIcon({
  kind,
  className = 'w-3.5 h-3.5 shrink-0',
}: {
  kind: ArtifactKind;
  className?: string;
}) {
  switch (kind) {
    case 'markdown':
      return <FileText className={className} />;
    case 'diff':
      return <GitMerge className={className} />;
    case 'json':
    case 'code':
      return <FileCode className={className} />;
    case 'worktree-ref':
      return <FileJson className={className} />;
    case 'text':
    case 'unknown':
    default:
      return <FileQuestion className={className} />;
  }
}
