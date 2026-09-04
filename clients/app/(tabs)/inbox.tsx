import { useCallback, useMemo, useState } from 'react';
import { View, Text, StyleSheet, Pressable } from 'react-native';
import { Ionicons } from '@expo/vector-icons';
import { Screen } from '@/components/layout/Screen';
import { theme } from '@/theme';

type Kind = 'pr' | 'issue' | 'message' | 'agent' | 'room';
type Filter = 'all' | 'mentions' | 'prs' | 'agents';

type Activity = {
  id: string;
  kind: Kind;
  actor: string;
  action: string;
  target: string;
  detail?: string;
  time: string;
  additions?: number;
  deletions?: number;
  meta?: string;
};

const MOCK: Activity[] = [
  {
    id: '1',
    kind: 'pr',
    actor: 'rick',
    action: 'opened a pull request',
    target: 'tt-node',
    detail: 'harden deploy tokens',
    additions: 128,
    deletions: 14,
    meta: 'ready for review',
    time: '12m',
  },
  {
    id: '2',
    kind: 'agent',
    actor: 'pearl',
    action: 'finished',
    target: 'Scarlet scan',
    detail: 'secrets/**',
    meta: '0 leaks · 2 redacted',
    time: '28m',
  },
  {
    id: '3',
    kind: 'message',
    actor: 'morty',
    action: 'mentioned you in',
    target: '#on-call',
    detail: 'can you take the inbound queue?',
    time: '1h',
  },
  {
    id: '4',
    kind: 'issue',
    actor: 'summer',
    action: 'labeled',
    target: 'turing-tarpit#42',
    meta: 'priority:high',
    time: '2h',
  },
  {
    id: '5',
    kind: 'room',
    actor: 'birdperson',
    action: 'posted in',
    target: '#Rust',
    detail: 'Firecracker vs CH for short workloads',
    time: '3h',
  },
  {
    id: '6',
    kind: 'pr',
    actor: 'summer',
    action: 'merged',
    target: 'tt-git',
    detail: 'smart-http range requests',
    additions: 41,
    deletions: 9,
    meta: 'into main',
    time: '5h',
  },
];

const KIND_ICON: Record<Kind, React.ComponentProps<typeof Ionicons>['name']> = {
  pr: 'git-pull-request-outline',
  issue: 'albums-outline',
  message: 'chatbubble-outline',
  agent: 'sparkles-outline',
  room: 'planet-outline',
};

const FILTERS: { id: Filter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'mentions', label: 'Mentions' },
  { id: 'prs', label: 'PRs' },
  { id: 'agents', label: 'Agents' },
];

function matchesFilter(item: Activity, filter: Filter): boolean {
  if (filter === 'all') return true;
  if (filter === 'mentions') return item.kind === 'message';
  if (filter === 'prs') return item.kind === 'pr';
  if (filter === 'agents') return item.kind === 'agent';
  return true;
}

function DiffStats({ additions, deletions }: { additions?: number; deletions?: number }) {
  if (additions == null && deletions == null) return null;
  return (
    <Text style={styles.diff}>
      {additions != null ? <Text style={styles.add}>+{additions}</Text> : null}
      {additions != null && deletions != null ? <Text style={styles.diffSep}> </Text> : null}
      {deletions != null ? <Text style={styles.del}>-{deletions}</Text> : null}
    </Text>
  );
}

export default function InboxScreen() {
  const [filter, setFilter] = useState<Filter>('all');
  const [refreshing, setRefreshing] = useState(false);

  const items = useMemo(
    () => MOCK.filter((a) => matchesFilter(a, filter)),
    [filter]
  );

  const onRefresh = useCallback(() => {
    setRefreshing(true);
    setTimeout(() => setRefreshing(false), 600);
  }, []);

  return (
    <Screen
      title="Inbox"
      subtitle="Activity across code, chat, and agents"
      refreshing={refreshing}
      onRefresh={onRefresh}
    >
      <View style={styles.filters}>
        {FILTERS.map((f) => {
          const active = filter === f.id;
          return (
            <Pressable
              key={f.id}
              onPress={() => setFilter(f.id)}
              style={[styles.chip, active && styles.chipActive]}
            >
              <Text style={[styles.chipText, active && styles.chipTextActive]}>{f.label}</Text>
            </Pressable>
          );
        })}
      </View>

      <View style={styles.list}>
        {items.length === 0 ? (
          <Text style={styles.empty}>Nothing in this filter.</Text>
        ) : (
          items.map((item) => (
            <Pressable key={item.id} style={styles.row}>
              <View style={styles.iconWrap}>
                <Ionicons name={KIND_ICON[item.kind]} size={18} color={theme.colors.orange} />
              </View>

              <View style={styles.rowBody}>
                <Text style={styles.line1} numberOfLines={2}>
                  <Text style={styles.actor}>{item.actor}</Text>
                  <Text style={styles.action}> {item.action} </Text>
                  <Text style={styles.target}>{item.target}</Text>
                </Text>

                {item.detail ? (
                  <Text style={styles.detail} numberOfLines={1}>
                    {item.detail}
                  </Text>
                ) : null}

                <View style={styles.metaRow}>
                  <DiffStats additions={item.additions} deletions={item.deletions} />
                  {item.meta ? (
                    <Text style={styles.meta} numberOfLines={1}>
                      {(item.additions != null || item.deletions != null) ? ' · ' : ''}
                      {item.meta}
                    </Text>
                  ) : null}
                </View>
              </View>

              <Text style={styles.time}>{item.time}</Text>
            </Pressable>
          ))
        )}
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  filters: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 8,
    marginBottom: theme.spacing.md,
  },
  chip: {
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderRadius: theme.radius.full,
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  chipActive: {
    borderColor: theme.colors.orange,
    backgroundColor: 'rgba(255, 60, 0, 0.12)',
  },
  chipText: {
    color: theme.colors.textMuted,
    fontSize: 13,
    fontWeight: '600',
  },
  chipTextActive: {
    color: theme.colors.orange,
  },
  list: {
    gap: 0,
  },
  empty: {
    color: theme.colors.textDim,
    textAlign: 'center',
    marginTop: 32,
    fontSize: 14,
  },
  row: {
    flexDirection: 'row',
    alignItems: 'flex-start',
    paddingVertical: 14,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: theme.colors.border,
    gap: 12,
  },
  iconWrap: {
    width: 36,
    height: 36,
    borderRadius: 10,
    backgroundColor: theme.colors.surfaceElevated,
    alignItems: 'center',
    justifyContent: 'center',
  },
  rowBody: {
    flex: 1,
    gap: 3,
    minWidth: 0,
  },
  line1: {
    fontSize: 15,
    lineHeight: 20,
    color: theme.colors.text,
  },
  actor: {
    fontWeight: '700',
    color: theme.colors.text,
  },
  action: {
    fontWeight: '400',
    color: theme.colors.textMuted,
  },
  target: {
    fontWeight: '600',
    color: theme.colors.orange,
  },
  detail: {
    color: theme.colors.text,
    fontSize: 14,
    lineHeight: 18,
  },
  metaRow: {
    flexDirection: 'row',
    alignItems: 'center',
    flexWrap: 'wrap',
    marginTop: 1,
  },
  diff: {
    fontSize: 12,
    fontWeight: '600',
    fontVariant: ['tabular-nums'],
  },
  add: {
    color: '#3fb950',
  },
  del: {
    color: '#f85149',
  },
  diffSep: {
    color: theme.colors.textDim,
  },
  meta: {
    color: theme.colors.textDim,
    fontSize: 12,
  },
  time: {
    color: theme.colors.textDim,
    fontSize: 12,
    marginTop: 2,
  },
});
