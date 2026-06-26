import React from 'react';

export interface TabDef {
  key: string;
  label: string;
  icon?: React.ReactNode;
}

interface TabBarProps {
  tabs: TabDef[];
  activeTab: string;
  onChange: (key: string) => void;
  className?: string;
}

export function TabBar({ tabs, activeTab, onChange, className = '' }: TabBarProps) {
  return (
    <div className={`flex gap-1 border-b border-white/5 pb-px ${className}`}>
      {tabs.map((tab) => (
        <button
          key={tab.key}
          onClick={() => onChange(tab.key)}
          className={`flex items-center gap-2 px-4 py-2.5 text-sm font-outfit font-medium border-b-2 transition-all ${
            activeTab === tab.key
              ? 'border-cyan-500 text-cyan-400'
              : 'border-transparent text-slate-400 hover:text-slate-200'
          }`}
        >
          {tab.icon}
          {tab.label}
        </button>
      ))}
    </div>
  );
}
