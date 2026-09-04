import { api } from './client';
import type { Channel, Message } from '@/types';

export async function listChannels(workspaceId: string) {
  const { data } = await api.get(`/api/workspaces/${workspaceId}/channels`);
  return data as Channel[];
}

export async function listMessages(channelId: string, cursor?: string) {
  const { data } = await api.get(`/api/channels/${channelId}/messages`, { params: { cursor } });
  return data as { items: Message[]; nextCursor?: string };
}

export async function postMessage(channelId: string, body: string, threadId?: string) {
  const { data } = await api.post(`/api/channels/${channelId}/messages`, { body, threadId });
  return data as Message;
}
