import { create } from 'zustand';
import type { Workspace } from '@/types';

interface WorkspaceState {
  current: Workspace | null;
  setCurrent: (w: Workspace | null) => void;
}

export const useWorkspaceStore = create<WorkspaceState>((set) => ({
  current: null,
  setCurrent: (current) => set({ current }),
}));
