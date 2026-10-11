import type { ReactNode } from 'react';

interface PlaceholderCardProps {
  title: string;
  children: ReactNode;
}

export function PlaceholderCard({ title, children }: PlaceholderCardProps) {
  return (
    <section className="glass-panel p-8">
      <h1 className="font-heading text-3xl font-bold text-white tracking-tight">{title}</h1>
      <div className="mt-3 text-sm text-slate-400">{children}</div>
    </section>
  );
}
