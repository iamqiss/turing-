import { api } from './client';
import type { Repository, PullRequest, Issue } from '@/types';

export async function listRepos(workspaceId: string) {
  const { data } = await api.get(`/api/workspaces/${workspaceId}/repos`);
  return data as Repository[];
}

export async function getRepo(workspace: string, repo: string) {
  const { data } = await api.get(`/api/repos/${workspace}/${repo}`);
  return data as Repository;
}

export async function listPulls(workspace: string, repo: string) {
  const { data } = await api.get(`/api/repos/${workspace}/${repo}/pulls`);
  return data as PullRequest[];
}

export async function listIssues(workspace: string, repo: string) {
  const { data } = await api.get(`/api/repos/${workspace}/${repo}/issues`);
  return data as Issue[];
}
