import React from 'react';

interface SectionCardProps {
  title: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  headerClassName?: string;
}

export function SectionCard({ title, icon, children, className = '', headerClassName = '' }: SectionCardProps) {
  return (
    <div className={`glass-panel p-5 ${className}`}>
      <h3 className={`font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2 ${headerClassName}`}>
        {icon}
        {title}
      </h3>
      {children}
    </div>
  );
}
