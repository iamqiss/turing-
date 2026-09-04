import { View, Text, StyleSheet } from 'react-native';
import { theme } from '@/theme';
import type { Message } from '@/types';

export function MessageBubble({ message, isMe }: { message: Message; isMe?: boolean }) {
  return (
    <View style={[styles.bubble, isMe ? styles.me : styles.them]}>
      <Text style={styles.body}>{message.body}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  bubble: { maxWidth: '80%', padding: 12, borderRadius: 12, marginVertical: 4 },
  me: { alignSelf: 'flex-end', backgroundColor: theme.colors.orange },
  them: { alignSelf: 'flex-start', backgroundColor: theme.colors.surfaceElevated },
  body: { color: '#fff' },
});
