import { api } from './client';
import type { Room } from '@/types';

export async function listRooms() {
  const { data } = await api.get('/api/rooms');
  return data as Room[];
}

export async function getRoom(roomId: string) {
  const { data } = await api.get(`/api/rooms/${roomId}`);
  return data as Room;
}
