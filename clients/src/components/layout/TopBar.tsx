import { View, Text, StyleSheet } from 'react-native';
import { theme } from '@/theme';

export function TopBar({ title }: { title: string }) {
  return (
    <View style={styles.bar}>
      <Text style={styles.title}>{title}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  bar: {
    height: 56,
    paddingHorizontal: 16,
    justifyContent: 'center',
    backgroundColor: theme.colors.pitchBlack,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: theme.colors.border,
  },
  title: { color: theme.colors.text, fontSize: 18, fontWeight: '600' },
});
