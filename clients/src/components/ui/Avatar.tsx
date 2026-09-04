import { Image, View, Text, StyleSheet } from 'react-native';
import { theme } from '@/theme';

type Props = { uri?: string; name: string; size?: number };

export function Avatar({ uri, name, size = 36 }: Props) {
  if (uri) {
    return <Image source={{ uri }} style={{ width: size, height: size, borderRadius: size / 2 }} />;
  }
  const initial = name.slice(0, 1).toUpperCase();
  return (
    <View style={[styles.fallback, { width: size, height: size, borderRadius: size / 2 }]}>
      <Text style={styles.initial}>{initial}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  fallback: { backgroundColor: theme.colors.surfaceElevated, alignItems: 'center', justifyContent: 'center' },
  initial: { color: theme.colors.orange, fontWeight: '700' },
});
