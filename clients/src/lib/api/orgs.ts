import { api } from './client';
import type { Org } from '@/types';

export async function listOrgs() {
  const { data } = await api.get('/api/orgs');
  return data as Org[];
}

export async function getOrg(orgId: string) {
  const { data } = await api.get(`/api/orgs/${orgId}`);
  return data as Org;
}
