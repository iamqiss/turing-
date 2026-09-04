import { Platform } from 'react-native';

function bufferToBase64Url(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let str = '';
  for (let i = 0; i < bytes.length; i++) {
    str += String.fromCharCode(bytes[i]);
  }
  const b64 = btoa(str);
  return b64.replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '');
}

function base64UrlToBuffer(b64url: string): ArrayBuffer {
  const padLen = (4 - (b64url.length % 4)) % 4;
  const padded = b64url + '='.repeat(padLen);
  const b64 = padded.replaceAll('-', '+').replaceAll('_', '/');
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) {
    bytes[i] = bin.charCodeAt(i);
  }
  return bytes.buffer;
}

function prepareCreateOptions(optionsJson: string): PublicKeyCredentialCreationOptions {
  const raw = typeof optionsJson === 'string' ? JSON.parse(optionsJson) : optionsJson;
  const o = raw.publicKey ?? raw;
  return {
    ...o,
    challenge: base64UrlToBuffer(o.challenge),
    user: {
      ...o.user,
      id: base64UrlToBuffer(o.user.id),
    },
    excludeCredentials: (o.excludeCredentials ?? []).map((c: { id: string; type: string; transports?: string[] }) => ({
      ...c,
      id: base64UrlToBuffer(c.id),
    })),
  };
}

function prepareRequestOptions(optionsJson: string): PublicKeyCredentialRequestOptions {
  const raw = typeof optionsJson === 'string' ? JSON.parse(optionsJson) : optionsJson;
  const o = raw.publicKey ?? raw;
  return {
    ...o,
    challenge: base64UrlToBuffer(o.challenge),
    allowCredentials: (o.allowCredentials ?? []).map((c: { id: string; type: string; transports?: string[] }) => ({
      ...c,
      id: base64UrlToBuffer(c.id),
    })),
  };
}

function credentialToJson(cred: PublicKeyCredential): string {
  const anyCred = cred as PublicKeyCredential & {
    response: AuthenticatorAttestationResponse & AuthenticatorAssertionResponse;
    authenticatorAttachment?: string;
    clientExtensionResults?: AuthenticationExtensionsClientOutputs;
  };
  const response = anyCred.response;
  const out: Record<string, unknown> = {
    id: cred.id,
    rawId: bufferToBase64Url(cred.rawId),
    type: cred.type,
    response: {} as Record<string, string>,
  };
  const resp = out.response as Record<string, string>;
  if (response.clientDataJSON) {
    resp.clientDataJSON = bufferToBase64Url(response.clientDataJSON);
  }
  if (response.attestationObject) {
    resp.attestationObject = bufferToBase64Url(response.attestationObject);
  }
  if (response.authenticatorData) {
    resp.authenticatorData = bufferToBase64Url(response.authenticatorData);
  }
  if (response.signature) {
    resp.signature = bufferToBase64Url(response.signature);
  }
  if (response.userHandle) {
    resp.userHandle = bufferToBase64Url(response.userHandle);
  }
  if (anyCred.authenticatorAttachment) {
    out.authenticatorAttachment = anyCred.authenticatorAttachment;
  }
  if (anyCred.clientExtensionResults) {
    out.clientExtensionResults = anyCred.clientExtensionResults;
  }
  return JSON.stringify(out);
}

export async function createPasskey(creationOptionsJson: string): Promise<string> {
  if (Platform.OS === 'web' && typeof navigator !== 'undefined' && navigator.credentials) {
    const options = prepareCreateOptions(creationOptionsJson);
    const cred = (await navigator.credentials.create({
      publicKey: options,
    })) as PublicKeyCredential | null;
    if (!cred) {
      throw new Error('Passkey creation cancelled');
    }
    return credentialToJson(cred);
  }
  throw new Error(
    'Passkeys on iOS/Android need react-native-passkey. Use Expo web for now, or add the native module.'
  );
}

export async function getPasskey(requestOptionsJson: string): Promise<string> {
  if (Platform.OS === 'web' && typeof navigator !== 'undefined' && navigator.credentials) {
    const options = prepareRequestOptions(requestOptionsJson);
    const cred = (await navigator.credentials.get({
      publicKey: options,
    })) as PublicKeyCredential | null;
    if (!cred) {
      throw new Error('Passkey authentication cancelled');
    }
    return credentialToJson(cred);
  }
  throw new Error(
    'Passkeys on iOS/Android need react-native-passkey. Use Expo web for now, or add the native module.'
  );
}

export function passkeysSupported(): boolean {
  if (Platform.OS === 'web') {
    return typeof window !== 'undefined' && typeof window.PublicKeyCredential !== 'undefined';
  }
  return true;
}
