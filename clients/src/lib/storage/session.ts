import * as SecureStore from 'expo-secure-store';

const KEY = 'tt_session';

export async function saveSession(token: string) {
  await SecureStore.setItemAsync(KEY, token);
}

export async function loadSession(): Promise<string | null> {
  return SecureStore.getItemAsync(KEY);
}

export async function clearSession() {
  await SecureStore.deleteItemAsync(KEY);
}
