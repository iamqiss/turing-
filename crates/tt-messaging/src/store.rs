//! Postgres persistence for channels and messages.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use tt_core::domain::{
    Channel, ChannelKind, Message, Reaction,
};
use tt_core::ids::{ChannelId, MessageId, PrincipalId, ThreadId, WorkspaceId};
use uuid::Uuid;

use crate::error::{MessagingError, Result};

#[async_trait]
pub trait MessageStore: Send + Sync {
    async fn create_channel(&self, channel: &Channel, member_ids: &[PrincipalId]) -> Result<()>;
    async fn get_channel(&self, id: ChannelId) -> Result<Option<Channel>>;
    async fn list_channels_for_principal(
        &self,
        workspace_id: WorkspaceId,
        principal_id: PrincipalId,
    ) -> Result<Vec<Channel>>;
    async fn add_member(&self, channel_id: ChannelId, principal_id: PrincipalId) -> Result<()>;
    async fn remove_member(&self, channel_id: ChannelId, principal_id: PrincipalId) -> Result<()>;
    async fn is_member(&self, channel_id: ChannelId, principal_id: PrincipalId) -> Result<bool>;

    async fn insert_message(&self, msg: &Message) -> Result<()>;
    async fn get_message(&self, id: MessageId) -> Result<Option<Message>>;
    async fn list_messages(
        &self,
        channel_id: ChannelId,
        thread_id: Option<ThreadId>,
        before: Option<MessageId>,
        limit: u32,
    ) -> Result<Vec<Message>>;
    async fn update_message_content(&self, id: MessageId, content: &str) -> Result<()>;
    async fn soft_delete_message(&self, id: MessageId) -> Result<()>;

    async fn add_reaction(&self, reaction: &Reaction) -> Result<()>;
    async fn remove_reaction(
        &self,
        message_id: MessageId,
        principal_id: PrincipalId,
        emoji: &str,
    ) -> Result<()>;
}

#[derive(Clone)]
pub struct PostgresMessageStore {
    pool: PgPool,
}

impl PostgresMessageStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct ChannelRow {
    id: Uuid,
    workspace_id: Uuid,
    kind: String,
    name: Option<String>,
    topic: Option<String>,
    is_archived: bool,
    created_by: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<ChannelRow> for Channel {
    fn from(r: ChannelRow) -> Self {
        let kind = match r.kind.as_str() {
            "private" => ChannelKind::Private,
            "direct" => ChannelKind::Direct,
            _ => ChannelKind::Public,
        };
        Channel {
            id: ChannelId::from_uuid(r.id),
            workspace_id: WorkspaceId::from_uuid(r.workspace_id),
            kind,
            name: r.name,
            topic: r.topic,
            is_archived: r.is_archived,
            created_by: PrincipalId::from_uuid(r.created_by),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(FromRow)]
struct MessageRow {
    id: Uuid,
    channel_id: Uuid,
    thread_id: Option<Uuid>,
    author_id: Uuid,
    content: String,
    reply_to_id: Option<Uuid>,
    is_edited: bool,
    is_deleted: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<MessageRow> for Message {
    fn from(r: MessageRow) -> Self {
        Message {
            id: MessageId::from_uuid(r.id),
            channel_id: ChannelId::from_uuid(r.channel_id),
            thread_id: r.thread_id.map(ThreadId::from_uuid),
            author_id: PrincipalId::from_uuid(r.author_id),
            content: r.content,
            reply_to_id: r.reply_to_id.map(MessageId::from_uuid),
            is_edited: r.is_edited,
            is_deleted: r.is_deleted,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

fn kind_str(k: ChannelKind) -> &'static str {
    match k {
        ChannelKind::Public => "public",
        ChannelKind::Private => "private",
        ChannelKind::Direct => "direct",
        ChannelKind::Thread => "private",
    }
}

#[async_trait]
impl MessageStore for PostgresMessageStore {
    async fn create_channel(&self, channel: &Channel, member_ids: &[PrincipalId]) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| MessagingError::Other(e.into()))?;

        sqlx::query(
            r#"
            INSERT INTO channels (
                id, workspace_id, kind, name, topic, is_archived, created_by, created_at, updated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
            "#,
        )
        .bind(channel.id.as_uuid())
        .bind(channel.workspace_id.as_uuid())
        .bind(kind_str(channel.kind))
        .bind(&channel.name)
        .bind(&channel.topic)
        .bind(channel.is_archived)
        .bind(channel.created_by.as_uuid())
        .bind(channel.created_at)
        .bind(channel.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;

        // Creator is always a member
        let mut members: Vec<PrincipalId> = member_ids.to_vec();
        if !members.contains(&channel.created_by) {
            members.push(channel.created_by);
        }

        for pid in members {
            sqlx::query(
                r#"
                INSERT INTO channel_members (channel_id, principal_id, joined_at)
                VALUES ($1, $2, NOW())
                ON CONFLICT DO NOTHING
                "#,
            )
            .bind(channel.id.as_uuid())
            .bind(pid.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        }

        tx.commit().await.map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn get_channel(&self, id: ChannelId) -> Result<Option<Channel>> {
        let row = sqlx::query_as::<_, ChannelRow>(
            r#"
            SELECT id, workspace_id, kind, name, topic, is_archived, created_by, created_at, updated_at
            FROM channels WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list_channels_for_principal(
        &self,
        workspace_id: WorkspaceId,
        principal_id: PrincipalId,
    ) -> Result<Vec<Channel>> {
        let rows = sqlx::query_as::<_, ChannelRow>(
            r#"
            SELECT c.id, c.workspace_id, c.kind, c.name, c.topic, c.is_archived,
                   c.created_by, c.created_at, c.updated_at
            FROM channels c
            LEFT JOIN channel_members m ON m.channel_id = c.id AND m.principal_id = $2
            WHERE c.workspace_id = $1
              AND c.is_archived = FALSE
              AND (c.kind = 'public' OR m.principal_id IS NOT NULL)
            ORDER BY c.name NULLS LAST, c.created_at
            "#,
        )
        .bind(workspace_id.as_uuid())
        .bind(principal_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn add_member(&self, channel_id: ChannelId, principal_id: PrincipalId) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO channel_members (channel_id, principal_id, joined_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(channel_id.as_uuid())
        .bind(principal_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn remove_member(&self, channel_id: ChannelId, principal_id: PrincipalId) -> Result<()> {
        sqlx::query(
            r#"DELETE FROM channel_members WHERE channel_id = $1 AND principal_id = $2"#,
        )
        .bind(channel_id.as_uuid())
        .bind(principal_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn is_member(&self, channel_id: ChannelId, principal_id: PrincipalId) -> Result<bool> {
        let row: (bool,) = sqlx::query_as(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM channel_members WHERE channel_id = $1 AND principal_id = $2
            )
            "#,
        )
        .bind(channel_id.as_uuid())
        .bind(principal_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.0)
    }

    async fn insert_message(&self, msg: &Message) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO messages (
                id, channel_id, thread_id, author_id, content, reply_to_id,
                is_edited, is_deleted, created_at, updated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
            "#,
        )
        .bind(msg.id.as_uuid())
        .bind(msg.channel_id.as_uuid())
        .bind(msg.thread_id.as_ref().map(|t| t.as_uuid()))
        .bind(msg.author_id.as_uuid())
        .bind(&msg.content)
        .bind(msg.reply_to_id.as_ref().map(|m| m.as_uuid()))
        .bind(msg.is_edited)
        .bind(msg.is_deleted)
        .bind(msg.created_at)
        .bind(msg.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn get_message(&self, id: MessageId) -> Result<Option<Message>> {
        let row = sqlx::query_as::<_, MessageRow>(
            r#"
            SELECT id, channel_id, thread_id, author_id, content, reply_to_id,
                   is_edited, is_deleted, created_at, updated_at
            FROM messages WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.map(Into::into))
    }

    async fn list_messages(
        &self,
        channel_id: ChannelId,
        thread_id: Option<ThreadId>,
        before: Option<MessageId>,
        limit: u32,
    ) -> Result<Vec<Message>> {
        let limit = limit.clamp(1, 100) as i64;
        let rows = if let Some(tid) = thread_id {
            sqlx::query_as::<_, MessageRow>(
                r#"
                SELECT id, channel_id, thread_id, author_id, content, reply_to_id,
                       is_edited, is_deleted, created_at, updated_at
                FROM messages
                WHERE channel_id = $1 AND thread_id = $2
                  AND ($3::uuid IS NULL OR created_at < (
                        SELECT created_at FROM messages WHERE id = $3
                      ))
                ORDER BY created_at DESC
                LIMIT $4
                "#,
            )
            .bind(channel_id.as_uuid())
            .bind(tid.as_uuid())
            .bind(before.as_ref().map(|m| *m.as_uuid()))
            .bind(limit)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query_as::<_, MessageRow>(
                r#"
                SELECT id, channel_id, thread_id, author_id, content, reply_to_id,
                       is_edited, is_deleted, created_at, updated_at
                FROM messages
                WHERE channel_id = $1 AND thread_id IS NULL
                  AND ($2::uuid IS NULL OR created_at < (
                        SELECT created_at FROM messages WHERE id = $2
                      ))
                ORDER BY created_at DESC
                LIMIT $3
                "#,
            )
            .bind(channel_id.as_uuid())
            .bind(before.as_ref().map(|m| *m.as_uuid()))
            .bind(limit)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| MessagingError::Other(e.into()))?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_message_content(&self, id: MessageId, content: &str) -> Result<()> {
        let res = sqlx::query(
            r#"
            UPDATE messages SET content = $2, is_edited = TRUE, updated_at = NOW()
            WHERE id = $1 AND is_deleted = FALSE
            "#,
        )
        .bind(id.as_uuid())
        .bind(content)
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        if res.rows_affected() == 0 {
            return Err(MessagingError::NotFound);
        }
        Ok(())
    }

    async fn soft_delete_message(&self, id: MessageId) -> Result<()> {
        let res = sqlx::query(
            r#"
            UPDATE messages SET is_deleted = TRUE, content = '', updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        if res.rows_affected() == 0 {
            return Err(MessagingError::NotFound);
        }
        Ok(())
    }

    async fn add_reaction(&self, reaction: &Reaction) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO reactions (message_id, principal_id, emoji, created_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(reaction.message_id.as_uuid())
        .bind(reaction.principal_id.as_uuid())
        .bind(&reaction.emoji)
        .bind(reaction.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn remove_reaction(
        &self,
        message_id: MessageId,
        principal_id: PrincipalId,
        emoji: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            DELETE FROM reactions
            WHERE message_id = $1 AND principal_id = $2 AND emoji = $3
            "#,
        )
        .bind(message_id.as_uuid())
        .bind(principal_id.as_uuid())
        .bind(emoji)
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }
}
