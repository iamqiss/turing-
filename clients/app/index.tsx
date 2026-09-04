import { Redirect } from 'expo-router';
import { Platform } from 'react-native';
import { useAuthStore } from '@/stores/auth';

export default function Index() {
  const token = useAuthStore((s) => s.token);

  // Mobile: don't block on passkeys until dev client + domain are ready
  if (Platform.OS !== 'web') {
    return <Redirect href="/(tabs)/inbox" />;
  }

  if (token) return <Redirect href="/(tabs)/inbox" />;
  return <Redirect href="/(auth)/login" />;
}
