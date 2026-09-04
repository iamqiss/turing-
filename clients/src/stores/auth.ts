import { create } from 'zustand';
import { loadSession, saveSession, clearSession } from '@/lib/storage/session';
import { getCurrentPrincipal, logout as apiLogout } from '@/lib/api/auth';
import type { Principal } from '@/types';

interface AuthState {
token: string | null;
principal: Principal | null;
hydrated: boolean;
busy: boolean;
error: string | null;
setSession: (token: string, principal?: Principal | null) => Promise<void>;
setPrincipal: (principal: Principal | null) => void;
setBusy: (busy: boolean) => void;
setError: (error: string | null) => void;
logout: () => Promise<void>;
hydrate: () => Promise<void>;
}

export const useAuthStore = create<AuthState>((set, get) => ({
token: null,
principal: null,
hydrated: false,
busy: false,
error: null,

setSession: async (token, principal = null) => {
await saveSession(token);
set({ token, principal, error: null });
},

setPrincipal: (principal) => set({ principal }),

setBusy: (busy) => set({ busy }),

setError: (error) => set({ error }),

logout: async () => {
try {
if (get().token) await apiLogout();
} catch {
/* still clear local */
}
await clearSession();
set({ token: null, principal: null, error: null });
},

hydrate: async () => {
try {
const token = await loadSession();
if (token) {
set({ token });
try {
const principal = await getCurrentPrincipal();
set({ principal });
} catch {
await clearSession();
set({ token: null, principal: null });
}
}
} finally {
set({ hydrated: true });
}
},
}));
