import React from 'react';

interface FieldLabelProps {
  children: React.ReactNode;
  icon?: React.ReactNode;
  htmlFor?: string;
  className?: string;
}

export function FieldLabel({ children, icon, htmlFor, className = '' }: FieldLabelProps) {
  return (
    <label
      htmlFor={htmlFor}
      className={`flex items-center gap-1.5 text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider ${className}`}
    >
      {icon}
      {children}
    </label>
  );
}
