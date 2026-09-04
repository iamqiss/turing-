import { View, Text } from 'react-native';
import { useLocalSearchParams } from 'expo-router';
export default function Screen() {
  const params = useLocalSearchParams();
  return (
    <View style={{ flex: 1, backgroundColor: '#0a0a0a', padding: 16 }}>
      <Text style={{ color: '#fff' }}>{JSON.stringify(params)}</Text>
    </View>
  );
}
