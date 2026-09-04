export type PrincipalId = string;
export type WorkspaceId = string;
export type ChannelId = string;
export type MessageId = string;
export type RepoId = string;
export type OrgId = string;
export type RoomId = string;
export type AgentId = string;

export interface Principal {
  id: PrincipalId;
  handle: string;
  displayName: string;
  avatarUrl?: string;
  kind: 'human' | 'agent';
}

export interface Workspace {
  id: WorkspaceId;
  slug: string;
  name: string;
  orgId?: OrgId;
}

export interface Channel {
  id: ChannelId;
  workspaceId: WorkspaceId;
  name: string;
  topic?: string;
  isPrivate: boolean;
}

export interface Message {
  id: MessageId;
  channelId: ChannelId;
  authorId: PrincipalId;
  body: string;
  createdAt: string;
  threadId?: MessageId;
  reactions?: { emoji: string; count: number; me: boolean }[];
}

export interface Repository {
  id: RepoId;
  workspaceId: WorkspaceId;
  name: string;
  description?: string;
  defaultBranch: string;
  isPrivate: boolean;
}

export interface PullRequest {
  id: string;
  number: number;
  title: string;
  state: 'open' | 'closed' | 'merged';
  authorId: PrincipalId;
  sourceBranch: string;
  targetBranch: string;
}

export interface Issue {
  id: string;
  number: number;
  title: string;
  state: 'open' | 'closed';
  authorId: PrincipalId;
  labels: string[];
}

export interface Org {
  id: OrgId;
  slug: string;
  name: string;
  seatCount: number;
}

export interface Room {
  id: RoomId;
  slug: string;
  name: string;
  description?: string;
  isPublic: boolean;
  memberCount: number;
}

export interface Agent {
  id: AgentId;
  name: string;
  status: 'idle' | 'running' | 'error';
  lastSeenAt?: string;
}
