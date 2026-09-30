import { create } from "zustand";

export type DragTarget = { type: "replace"; slot: number } | { type: "gap"; gap: number } | null;

export interface DragState {
  active: boolean;
  source: "library" | "bank" | null;
  libIds: string[];
  bankSlots: number[];
  x: number;
  y: number;
  target: DragTarget;
  set: (p: Partial<DragState>) => void;
  reset: () => void;
}

export const useDrag = create<DragState>((set) => ({
  active: false,
  source: null,
  libIds: [],
  bankSlots: [],
  x: 0,
  y: 0,
  target: null,
  set: (p) => set(p),
  reset: () => set({ active: false, source: null, libIds: [], bankSlots: [], target: null }),
}));
