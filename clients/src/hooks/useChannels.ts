import { useQuery } from '@tanstack/react-query';
import { listChannels } from '@/lib/api';

export function useChannels(workspaceId: string | undefined) {
  return useQuery({
    queryKey: ['channels', workspaceId],
    queryFn: () => listChannels(workspaceId!),
    enabled: !!workspaceId,
  });
}
