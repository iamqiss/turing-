import { Redirect } from 'expo-router';

/** Registration lives on the same screen as sign-in (mode toggle). */
export default function RegisterRedirect() {
  return <Redirect href="/(auth)/login" />;
}
