import { formatDistanceToNow } from 'date-fns';

export function timeAgo(iso: string) {
  return formatDistanceToNow(new Date(iso), { addSuffix: true });
}
