// Reusable status badge, isolated from any domain logic: callers decide
// which tone communicates which state.

import type { ReactNode } from "react";

export type BadgeTone = "ok" | "warn" | "error" | "neutral";

const TONE_CLASSES: Record<BadgeTone, string> = {
  ok: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/30",
  warn: "bg-amber-500/15 text-amber-300 ring-amber-500/30",
  error: "bg-red-500/15 text-red-300 ring-red-500/30",
  neutral: "bg-slate-500/15 text-slate-300 ring-slate-500/30",
};

export interface BadgeProps {
  tone: BadgeTone;
  children: ReactNode;
  /** Optional tooltip explaining the badge meaning. */
  title?: string;
}

export default function Badge({ tone, children, title }: BadgeProps) {
  return (
    <span
      title={title}
      className={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${TONE_CLASSES[tone]}`}
    >
      {children}
    </span>
  );
}
