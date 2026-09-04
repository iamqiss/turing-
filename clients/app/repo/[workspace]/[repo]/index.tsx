import { View, Text } from 'react-native';
import { useLocalSearchParams } from 'expo-router';
export default function RepoHome() {
  const { workspace, repo } = useLocalSearchParams<{ workspace: string; repo: string }>();
  return (
    <View style={{ flex: 1, backgroundColor: '#0a0a0a', padding: 16 }}>
      <Text style={{ color: '#fff' }}>{workspace}/{repo}</Text>
    </View>
  );
}
