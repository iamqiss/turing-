import { View, Text } from 'react-native';
import { theme } from '@/theme';
export default function ProjectsScreen() {
  return (
    <View style={{ flex: 1, backgroundColor: theme.colors.pitchBlack, padding: 16 }}>
      <Text style={{ color: '#fff', fontSize: 22, fontWeight: '600' }}>Projects</Text>
    </View>
  );
}
