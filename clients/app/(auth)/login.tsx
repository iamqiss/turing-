import { useState } from 'react';
import {
  View,
  Text,
  TextInput,
  Pressable,
  StyleSheet,
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  ScrollView,
  type TextInputProps,
} from 'react-native';
import { router } from 'expo-router';
import { theme } from '@/theme';
import {
  beginRegistration,
  finishRegistration,
  beginAuthentication,
  finishAuthentication,
  getCurrentPrincipal,
} from '@/lib/api/auth';
import { createPasskey, getPasskey, passkeysSupported } from '@/lib/auth/passkey';
import { useAuthStore } from '@/stores/auth';

type Mode = 'signin' | 'register';

export default function AuthScreen() {
  const [mode, setMode] = useState<Mode>('signin');
  const [handle, setHandle] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [email, setEmail] = useState('');

  const setSession = useAuthStore((s) => s.setSession);
  const setPrincipal = useAuthStore((s) => s.setPrincipal);
  const busy = useAuthStore((s) => s.busy);
  const setBusy = useAuthStore((s) => s.setBusy);
  const error = useAuthStore((s) => s.error);
  const setError = useAuthStore((s) => s.setError);

  const canSubmit =
    handle.trim().length >= 2 &&
    (mode === 'signin' || displayName.trim().length >= 1) &&
    !busy;

  async function runCeremony() {
    setError(null);
    setBusy(true);
    try {
      if (!passkeysSupported()) {
        throw new Error('Passkeys are not available on this device/browser.');
      }

      if (mode === 'register') {
        const { publicKeyCredentialCreationOptions, ceremonyId } = await beginRegistration({
          displayName: displayName.trim(),
          handle: handle.trim(),
          email: email.trim() || undefined,
        });
        const credentialJson = await createPasskey(publicKeyCredentialCreationOptions);
        const { sessionToken } = await finishRegistration(ceremonyId, credentialJson);
        await setSession(sessionToken);
        const principal = await getCurrentPrincipal();
        setPrincipal(principal);
      } else {
        const { publicKeyCredentialRequestOptions, ceremonyId } = await beginAuthentication(
          handle.trim()
        );
        const credentialJson = await getPasskey(publicKeyCredentialRequestOptions);
        const { sessionToken } = await finishAuthentication(ceremonyId, credentialJson);
        await setSession(sessionToken);
        const principal = await getCurrentPrincipal();
        setPrincipal(principal);
      }

      router.replace('/(tabs)/inbox');
    } catch (e: unknown) {
      let msg = 'Ceremony failed. Check handle and try again.';
      if (e && typeof e === 'object') {
        const anyErr = e as {
          message?: string;
          response?: { data?: { message?: string } };
        };
        msg = anyErr.response?.data?.message || anyErr.message || msg;
      }
      setError(String(msg));
    } finally {
      setBusy(false);
    }
  }

  return (
    <KeyboardAvoidingView
      style={styles.root}
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
    >
      <ScrollView
        contentContainerStyle={styles.scroll}
        keyboardShouldPersistTaps="handled"
        showsVerticalScrollIndicator={false}
      >
        <View style={styles.brand}>
          <Text style={styles.logoMark}>TT</Text>
          <Text style={styles.logo}>Turing Tarpit</Text>
          <Text style={styles.tagline}>Developer OS · humans & agents</Text>
        </View>

        <View style={styles.segment}>
          <Pressable
            onPress={() => {
              setMode('signin');
              setError(null);
            }}
            style={[styles.segBtn, mode === 'signin' && styles.segBtnActive]}
          >
            <Text style={[styles.segText, mode === 'signin' && styles.segTextActive]}>
              Sign in
            </Text>
          </Pressable>
          <Pressable
            onPress={() => {
              setMode('register');
              setError(null);
            }}
            style={[styles.segBtn, mode === 'register' && styles.segBtnActive]}
          >
            <Text style={[styles.segText, mode === 'register' && styles.segTextActive]}>
              Create account
            </Text>
          </Pressable>
        </View>

        <View style={styles.form}>
          {mode === 'register' ? (
            <Field
              label="Display name"
              value={displayName}
              onChangeText={setDisplayName}
              placeholder="Rick Sanchez"
              autoComplete="name"
            />
          ) : null}

          <Field
            label="Handle"
            value={handle}
            onChangeText={setHandle}
            placeholder="rick"
            autoCapitalize="none"
            autoCorrect={false}
            autoComplete="username"
          />

          {mode === 'register' ? (
            <Field
              label="Email (optional)"
              value={email}
              onChangeText={setEmail}
              placeholder="rick@citadel.dev"
              autoCapitalize="none"
              keyboardType="email-address"
              autoComplete="email"
            />
          ) : null}

          {error ? (
            <View style={styles.errorBox}>
              <Text style={styles.errorText}>{error}</Text>
            </View>
          ) : null}

          <Pressable
            onPress={runCeremony}
            disabled={!canSubmit}
            style={[styles.cta, !canSubmit && styles.ctaDisabled]}
          >
            {busy ? (
              <ActivityIndicator color="#000000" />
            ) : (
              <Text style={styles.ctaLabel}>
                {mode === 'register' ? 'Create passkey' : 'Sign in with passkey'}
              </Text>
            )}
          </Pressable>

          {Platform.OS !== 'web' ? (
            <Pressable
              onPress={() => router.replace('/(tabs)/inbox')}
              style={[styles.cta, { backgroundColor: theme.colors.surfaceElevated, marginTop: 8 }]}
            >
              <Text style={[styles.ctaLabel, { color: theme.colors.text }]}>
                Continue without passkey (dev)
              </Text>
            </Pressable>
          ) : null}

          <Text style={styles.hint}>
            {mode === 'register'
              ? 'A passkey will be created on this device. No password.'
              : 'Uses a passkey already registered for this handle.'}
          </Text>
        </View>
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

type FieldProps = TextInputProps & { label: string };

function Field({ label, onFocus, onBlur, ...props }: FieldProps) {
  const [focused, setFocused] = useState(false);
  return (
    <View style={styles.field}>
      <Text style={styles.label}>{label}</Text>
      <TextInput
        {...props}
        placeholderTextColor={theme.colors.textDim}
        onFocus={(e) => {
          setFocused(true);
          onFocus?.(e);
        }}
        onBlur={(e) => {
          setFocused(false);
          onBlur?.(e);
        }}
        style={[styles.input, focused && styles.inputFocused]}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
    backgroundColor: theme.colors.background,
  },
  scroll: {
    flexGrow: 1,
    justifyContent: 'center',
    paddingHorizontal: theme.spacing.lg,
    paddingVertical: theme.spacing.xxl,
  },
  brand: {
    alignItems: 'center',
    marginBottom: theme.spacing.xl,
  },
  logoMark: {
    color: theme.colors.orange,
    fontSize: 28,
    fontWeight: '800',
    letterSpacing: 2,
    marginBottom: 8,
  },
  logo: {
    color: theme.colors.text,
    fontSize: 32,
    fontWeight: '700',
    letterSpacing: -0.5,
  },
  tagline: {
    marginTop: 6,
    color: theme.colors.textMuted,
    fontSize: 13,
  },
  segment: {
    flexDirection: 'row',
    backgroundColor: theme.colors.surface,
    borderRadius: theme.radius.md,
    padding: 4,
    marginBottom: theme.spacing.lg,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: theme.colors.border,
  },
  segBtn: {
    flex: 1,
    paddingVertical: 10,
    borderRadius: theme.radius.sm,
    alignItems: 'center',
  },
  segBtnActive: {
    backgroundColor: theme.colors.surfaceElevated,
  },
  segText: {
    color: theme.colors.textMuted,
    fontWeight: '600',
    fontSize: 14,
  },
  segTextActive: {
    color: theme.colors.orange,
  },
  form: {
    gap: theme.spacing.md,
  },
  field: {
    gap: 6,
  },
  label: {
    color: theme.colors.textMuted,
    fontSize: 12,
    fontWeight: '600',
    letterSpacing: 0.4,
    textTransform: 'uppercase',
  },
  input: {
    backgroundColor: theme.colors.inputBg,
    borderWidth: 1,
    borderColor: theme.colors.inputBorder,
    borderRadius: theme.radius.md,
    paddingHorizontal: 14,
    paddingVertical: Platform.OS === 'ios' ? 14 : 12,
    color: theme.colors.text,
    fontSize: 16,
  },
  inputFocused: {
    borderColor: theme.colors.inputBorderFocus,
  },
  errorBox: {
    backgroundColor: 'rgba(239, 68, 68, 0.12)',
    borderRadius: theme.radius.sm,
    padding: 12,
    borderWidth: 1,
    borderColor: 'rgba(239, 68, 68, 0.35)',
  },
  errorText: {
    color: theme.colors.danger,
    fontSize: 14,
  },
  cta: {
    backgroundColor: theme.colors.orange,
    borderRadius: theme.radius.md,
    paddingVertical: 16,
    alignItems: 'center',
    marginTop: 4,
  },
  ctaDisabled: {
    opacity: 0.4,
  },
  ctaLabel: {
    color: '#000000',
    fontWeight: '700',
    fontSize: 16,
  },
  hint: {
    textAlign: 'center',
    color: theme.colors.textDim,
    fontSize: 13,
    marginTop: 4,
  },
});
