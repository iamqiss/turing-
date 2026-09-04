import { useEffect, type ReactNode } from 'react';
import { Platform } from 'react-native';
import { Stack } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { GestureHandlerRootView } from 'react-native-gesture-handler';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { theme } from '@/theme';
import { useAuthStore } from '@/stores/auth';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: 1, staleTime: 30_000 },
  },
});

function AuthHydration({ children }: { children: ReactNode }) {
  const hydrate = useAuthStore((s) => s.hydrate);
  const hydrated = useAuthStore((s) => s.hydrated);

  useEffect(() => {
    hydrate();
  }, [hydrate]);

  // On native during dev, don't block the tree if session hydrate is slow
  if (!hydrated && Platform.OS === 'web') return null;
  return <>{children}</>;
}

export default function RootLayout() {
  return (
    <SafeAreaProvider>
    <GestureHandlerRootView style={{ flex: 1, backgroundColor: theme.colors.background }}>
      <QueryClientProvider client={queryClient}>
        <StatusBar style="light" />
        <AuthHydration>
          <Stack
            screenOptions={{
              headerShown: false,
              contentStyle: { backgroundColor: theme.colors.background },
              animation: 'fade',
            }}
          />
        </AuthHydration>
      </QueryClientProvider>
    </GestureHandlerRootView>
    </SafeAreaProvider>
  );
}
