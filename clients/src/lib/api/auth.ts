import { api } from './client';
import type { Principal } from '@/types';

const AUTH = '/turing.tarpit.auth.v1.AuthService';

export type BeginRegistrationResult = {
  publicKeyCredentialCreationOptions: string;
  ceremonyId: string;
};

export type BeginAuthenticationResult = {
  publicKeyCredentialRequestOptions: string;
  ceremonyId: string;
};

export type AuthSuccess = {
  principalId: string;
  sessionToken: string;
};

function mapPrincipal(raw: any): Principal {
  return {
    id: raw?.id?.value ?? raw?.id ?? '',
    handle: raw?.handle ?? '',
    displayName: raw?.display_name ?? raw?.displayName ?? '',
    avatarUrl: raw?.avatar_url ?? raw?.avatarUrl,
    kind: (raw?.kind === 'agent' ? 'agent' : 'human') as 'human' | 'agent',
  };
}

export async function beginRegistration(input: {
  displayName: string;
  handle: string;
  email?: string;
}): Promise<BeginRegistrationResult> {
  const url = AUTH + '/BeginRegistration';
  const { data } = await api.post(url, {
    display_name: input.displayName.trim(),
    handle: input.handle.trim().toLowerCase(),
    email: input.email?.trim() ?? '',
  });
  return {
    publicKeyCredentialCreationOptions:
      data.publicKeyCredentialCreationOptions ??
      data.public_key_credential_creation_options,
    ceremonyId: data.ceremonyId ?? data.ceremony_id,
  };
}

export async function finishRegistration(
  ceremonyId: string,
  publicKeyCredentialJson: string
): Promise<AuthSuccess> {
  const url = AUTH + '/FinishRegistration';
  const { data } = await api.post(url, {
    ceremony_id: ceremonyId,
    public_key_credential: publicKeyCredentialJson,
  });
  return {
    principalId:
      data.principal_id?.value ??
      data.principalId?.value ??
      data.principal_id ??
      '',
    sessionToken: data.session_token ?? data.sessionToken,
  };
}

export async function beginAuthentication(
  handle: string
): Promise<BeginAuthenticationResult> {
  const url = AUTH + '/BeginAuthentication';
  const { data } = await api.post(url, {
    handle: handle.trim().toLowerCase(),
  });
  return {
    publicKeyCredentialRequestOptions:
      data.publicKeyCredentialRequestOptions ??
      data.public_key_credential_request_options,
    ceremonyId: data.ceremonyId ?? data.ceremony_id,
  };
}

export async function finishAuthentication(
  ceremonyId: string,
  publicKeyCredentialJson: string
): Promise<AuthSuccess> {
  const url = AUTH + '/FinishAuthentication';
  const { data } = await api.post(url, {
    ceremony_id: ceremonyId,
    public_key_credential: publicKeyCredentialJson,
  });
  return {
    principalId:
      data.principal_id?.value ??
      data.principalId?.value ??
      data.principal_id ??
      '',
    sessionToken: data.session_token ?? data.sessionToken,
  };
}

export async function getCurrentPrincipal(): Promise<Principal> {
  const url = AUTH + '/GetCurrentPrincipal';
  const { data } = await api.post(url, {});
  return mapPrincipal(data.principal);
}

export async function logout(): Promise<void> {
  const url = AUTH + '/Logout';
  await api.post(url, {});
}
