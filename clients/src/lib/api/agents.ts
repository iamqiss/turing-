import { api } from './client';
import type { Agent } from '@/types';

export async function listAgents() {
  const { data } = await api.get('/api/agents');
  return data as Agent[];
}
