import { RailNavItem } from 'demeteo';
import { Terminal, Layers, Server, Settings } from 'lucide-react';

/** Expanded rail — the width the project rail uses by default. */
export const Expanded = () => (
  <div className="w-60 flex flex-col gap-1.5 p-3 rounded-xl bg-[#0d0f14] border border-white/5">
    <RailNavItem icon={Layers} label="Features" active onClick={() => {}} />
    <RailNavItem icon={Terminal} label="Terminals" count={4} pulse onClick={() => {}} />
    <RailNavItem icon={Server} label="Machines" count={2} onClick={() => {}} />
    <RailNavItem icon={Settings} label="Settings" onClick={() => {}} />
  </div>
);

/** The attention badge — ruby, pulsing, for terminals needing a decision. */
export const NeedsAttention = () => (
  <div className="w-60 flex flex-col gap-1.5 p-3 rounded-xl bg-[#0d0f14] border border-white/5">
    <RailNavItem icon={Terminal} label="Terminals" count={6} attentionCount={2} pulse onClick={() => {}} />
    <RailNavItem icon={Layers} label="Features" count={3} onClick={() => {}} />
  </div>
);

/** Collapsed rail — icon only, count in the corner, pulse bottom-right. */
export const Collapsed = () => (
  <div className="w-14 flex flex-col items-center gap-2 p-2 rounded-xl bg-[#0d0f14] border border-white/5">
    <RailNavItem icon={Layers} label="Features" active collapsed onClick={() => {}} />
    <RailNavItem icon={Terminal} label="Terminals" count={4} pulse collapsed onClick={() => {}} />
    <RailNavItem icon={Server} label="Machines" attentionCount={1} collapsed onClick={() => {}} />
    <RailNavItem icon={Settings} label="Settings" collapsed onClick={() => {}} />
  </div>
);

/** Active vs idle, side by side — the only difference is the surface tint. */
export const ActiveVsIdle = () => (
  <div className="w-60 flex flex-col gap-1.5">
    <RailNavItem icon={Layers} label="Active item" active onClick={() => {}} />
    <RailNavItem icon={Layers} label="Idle item" onClick={() => {}} />
  </div>
);
