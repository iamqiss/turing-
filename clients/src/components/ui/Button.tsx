import { Pressable, Text, StyleSheet, ViewStyle } from 'react-native';
import { theme } from '@/theme';

type Props = {
  title: string;
  onPress: () => void;
  variant?: 'primary' | 'ghost' | 'danger';
  style?: ViewStyle;
};

export function Button({ title, onPress, variant = 'primary', style }: Props) {
  const bg =
    variant === 'primary' ? theme.colors.orange :
    variant === 'danger' ? theme.colors.danger : 'transparent';
  return (
    <Pressable onPress={onPress} style={[styles.base, { backgroundColor: bg }, style]}>
      <Text style={styles.label}>{title}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  base: {
    paddingVertical: 12,
    paddingHorizontal: 20,
    borderRadius: theme.radius.md,
    alignItems: 'center',
  },
  label: { color: '#fff', fontWeight: '600', fontSize: 16 },
});
