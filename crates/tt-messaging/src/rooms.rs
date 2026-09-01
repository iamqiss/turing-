//! Rooms — public/discoverable community feeds.
//! Participants are **Roomies**. Live multi-party video is deferred;
//! posts support text, voice notes, short clips, and attachments via media.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;
use tracing::info;
use tt_core::domain::{Room, RoomPost, RoomPostKind, RoomVisibility};
use tt_core::ids::{PrincipalId, RoomId, RoomPostId};
use uuid::Uuid;

use crate::error::{MessagingError, Result};

const MAX_TITLE: usize = 120;
const MAX_DESCRIPTION: usize = 2_000;
const MAX_POST_CONTENT: usize = 8_000;
const MAX_SLUG: usize = 64;
const MIN_SLUG: usize = 2;

/// Reserved so community rooms cannot shadow product routes / system names.
const RESERVED_SLUGS: &[&str] = &[
    "api", "admin", "mine", "new", "settings", "me", "null", "undefined",
    "www", "static", "assets", "healthz", "readyz", "login", "logout",
    "rooms", "room", "channel", "channels", "dm", "system", "tt", "turing",
];

fn slugify(raw: &str) -> Result<String> {
    let s = raw
        .trim()
        .trim_start_matches('#')
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    // collapse repeated dashes
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.chars() {
        if c == '-' {
            if prev_dash {
                continue;
            }
            prev_dash = true;
            out.push(c);
        } else {
            prev_dash = false;
            out.push(c);
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.len() < MIN_SLUG || s.len() > MAX_SLUG {
        return Err(MessagingError::InvalidArgument(format!(
            "slug must be {MIN_SLUG}–{MAX_SLUG} chars [a-z0-9_-]"
        )));
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Err(MessagingError::InvalidArgument(
            "slug cannot be purely numeric".into(),
        ));
    }
    if RESERVED_SLUGS.contains(&s.as_str()) {
        return Err(MessagingError::InvalidArgument(format!(
            "slug '{s}' is reserved"
        )));
    }
    Ok(s)
}

fn sanitize_text(s: &str, max: usize) -> Result<String> {
    let t = s.trim();
    if t.chars().count() > max {
        return Err(MessagingError::InvalidArgument(format!(
            "text exceeds {max} characters"
        )));
    }
    // strip NULs and other control chars except newline/tab
    let cleaned: String = t
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect();
    Ok(cleaned)
}

fn parse_visibility(s: &str) -> Result<RoomVisibility> {
    match s {
        "public" => Ok(RoomVisibility::Public),
        "private" => Ok(RoomVisibility::Private),
        _ => Err(MessagingError::InvalidArgument("visibility must be public|private".into())),
    }
}

fn vis_str(v: RoomVisibility) -> &'static str {
    match v {
        RoomVisibility::Public => "public",
        RoomVisibility::Private => "private",
    }
}

fn parse_kind(s: &str) -> Result<RoomPostKind> {
    match s {
        "text" => Ok(RoomPostKind::Text),
        "voice_note" => Ok(RoomPostKind::VoiceNote),
        "video_clip" => Ok(RoomPostKind::VideoClip),
        "attachment" => Ok(RoomPostKind::Attachment),
        _ => Err(MessagingError::InvalidArgument(
            "kind must be text|voice_note|video_clip|attachment".into(),
        )),
    }
}

fn kind_str(k: RoomPostKind) -> &'static str {
    match k {
        RoomPostKind::Text => "text",
        RoomPostKind::VoiceNote => "voice_note",
        RoomPostKind::VideoClip => "video_clip",
        RoomPostKind::Attachment => "attachment",
    }
}

#[async_trait]
pub trait RoomStore: Send + Sync {
    async fn insert_room(&self, room: &Room, owner: PrincipalId) -> Result<()>;
    async fn get_by_slug(&self, slug: &str) -> Result<Option<Room>>;
    async fn get_by_id(&self, id: RoomId) -> Result<Option<Room>>;
    async fn list_discoverable(&self, limit: i64, offset: i64) -> Result<Vec<Room>>;
    async fn list_joined(&self, principal: PrincipalId) -> Result<Vec<Room>>;
    async fn is_member(&self, room_id: RoomId, principal: PrincipalId) -> Result<bool>;
    async fn join(&self, room_id: RoomId, principal: PrincipalId, role: &str) -> Result<()>;
    async fn leave(&self, room_id: RoomId, principal: PrincipalId) -> Result<()>;
    async fn insert_post(&self, post: &RoomPost) -> Result<()>;
    async fn list_posts(
        &self,
        room_id: RoomId,
        before: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<RoomPost>>;
}

pub struct PostgresRoomStore {
    pool: PgPool,
}

impl PostgresRoomStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn map_room(
        id: Uuid,
        slug: String,
        title: String,
        description: Option<String>,
        visibility: String,
        created_by: Uuid,
        member_count: i32,
        is_archived: bool,
        created_at: chrono::DateTime<Utc>,
        updated_at: chrono::DateTime<Utc>,
    ) -> Result<Room> {
        Ok(Room {
            id: RoomId::from_uuid(id),
            slug,
            title,
            description,
            visibility: parse_visibility(&visibility)?,
            created_by: PrincipalId::from_uuid(created_by),
            member_count: member_count.max(0) as u32,
            is_archived,
            created_at,
            updated_at,
        })
    }

    fn map_post(
        id: Uuid,
        room_id: Uuid,
        author_id: Uuid,
        kind: String,
        content: String,
        media_id: Option<Uuid>,
        is_edited: bool,
        is_deleted: bool,
        created_at: chrono::DateTime<Utc>,
        updated_at: chrono::DateTime<Utc>,
    ) -> Result<RoomPost> {
        Ok(RoomPost {
            id: RoomPostId::from_uuid(id),
            room_id: RoomId::from_uuid(room_id),
            author_id: PrincipalId::from_uuid(author_id),
            kind: parse_kind(&kind)?,
            content,
            media_id,
            is_edited,
            is_deleted,
            created_at,
            updated_at,
        })
    }
}

#[async_trait]
impl RoomStore for PostgresRoomStore {
    async fn insert_room(&self, room: &Room, owner: PrincipalId) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| MessagingError::Other(e.into()))?;
        sqlx::query(
            r#"
            INSERT INTO rooms (id, slug, title, description, visibility, created_by, member_count, is_archived, created_at, updated_at)
            VALUES ($1,$2,$3,$4,$5,$6,1,FALSE,$7,$8)
            "#,
        )
        .bind(room.id.as_uuid())
        .bind(&room.slug)
        .bind(&room.title)
        .bind(&room.description)
        .bind(vis_str(room.visibility))
        .bind(room.created_by.as_uuid())
        .bind(room.created_at)
        .bind(room.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            if e.to_string().contains("rooms_slug") {
                MessagingError::Conflict("slug already taken".into())
            } else {
                MessagingError::Other(e.into())
            }
        })?;
        sqlx::query(
            "INSERT INTO room_members (room_id, principal_id, role, joined_at) VALUES ($1,$2,'owner',$3)",
        )
        .bind(room.id.as_uuid())
        .bind(owner.as_uuid())
        .bind(room.created_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        tx.commit().await.map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn get_by_slug(&self, slug: &str) -> Result<Option<Room>> {
        let row: Option<(Uuid, String, String, Option<String>, String, Uuid, i32, bool, chrono::DateTime<Utc>, chrono::DateTime<Utc>)> =
            sqlx::query_as(
                r#"SELECT id, slug, title, description, visibility, created_by, member_count, is_archived, created_at, updated_at
                   FROM rooms WHERE lower(slug) = lower($1)"#,
            )
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        row.map(|(a, b, c, d, e, f, g, h, i, j)| Self::map_room(a, b, c, d, e, f, g, h, i, j))
            .transpose()
    }

    async fn get_by_id(&self, id: RoomId) -> Result<Option<Room>> {
        let row: Option<(Uuid, String, String, Option<String>, String, Uuid, i32, bool, chrono::DateTime<Utc>, chrono::DateTime<Utc>)> =
            sqlx::query_as(
                r#"SELECT id, slug, title, description, visibility, created_by, member_count, is_archived, created_at, updated_at
                   FROM rooms WHERE id = $1"#,
            )
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        row.map(|(a, b, c, d, e, f, g, h, i, j)| Self::map_room(a, b, c, d, e, f, g, h, i, j))
            .transpose()
    }

    async fn list_discoverable(&self, limit: i64, offset: i64) -> Result<Vec<Room>> {
        let rows: Vec<(Uuid, String, String, Option<String>, String, Uuid, i32, bool, chrono::DateTime<Utc>, chrono::DateTime<Utc>)> =
            sqlx::query_as(
                r#"SELECT id, slug, title, description, visibility, created_by, member_count, is_archived, created_at, updated_at
                   FROM rooms
                   WHERE visibility = 'public' AND NOT is_archived
                   ORDER BY member_count DESC, created_at DESC
                   LIMIT $1 OFFSET $2"#,
            )
            .bind(limit.clamp(1, 100))
            .bind(offset.max(0))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        rows.into_iter()
            .map(|(a, b, c, d, e, f, g, h, i, j)| Self::map_room(a, b, c, d, e, f, g, h, i, j))
            .collect()
    }

    async fn list_joined(&self, principal: PrincipalId) -> Result<Vec<Room>> {
        let rows: Vec<(Uuid, String, String, Option<String>, String, Uuid, i32, bool, chrono::DateTime<Utc>, chrono::DateTime<Utc>)> =
            sqlx::query_as(
                r#"SELECT r.id, r.slug, r.title, r.description, r.visibility, r.created_by, r.member_count, r.is_archived, r.created_at, r.updated_at
                   FROM rooms r
                   INNER JOIN room_members m ON m.room_id = r.id
                   WHERE m.principal_id = $1 AND NOT r.is_archived
                   ORDER BY r.title"#,
            )
            .bind(principal.as_uuid())
            .fetch_all(&self.pool)
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        rows.into_iter()
            .map(|(a, b, c, d, e, f, g, h, i, j)| Self::map_room(a, b, c, d, e, f, g, h, i, j))
            .collect()
    }

    async fn is_member(&self, room_id: RoomId, principal: PrincipalId) -> Result<bool> {
        let row: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM room_members WHERE room_id = $1 AND principal_id = $2)",
        )
        .bind(room_id.as_uuid())
        .bind(principal.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(row.0)
    }

    async fn join(&self, room_id: RoomId, principal: PrincipalId, role: &str) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| MessagingError::Other(e.into()))?;
        let res = sqlx::query(
            r#"INSERT INTO room_members (room_id, principal_id, role, joined_at)
               VALUES ($1,$2,$3,NOW())
               ON CONFLICT DO NOTHING"#,
        )
        .bind(room_id.as_uuid())
        .bind(principal.as_uuid())
        .bind(role)
        .execute(&mut *tx)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        if res.rows_affected() > 0 {
            sqlx::query("UPDATE rooms SET member_count = member_count + 1, updated_at = NOW() WHERE id = $1")
                .bind(room_id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(|e| MessagingError::Other(e.into()))?;
        }
        tx.commit().await.map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn leave(&self, room_id: RoomId, principal: PrincipalId) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(|e| MessagingError::Other(e.into()))?;
        let res = sqlx::query(
            "DELETE FROM room_members WHERE room_id = $1 AND principal_id = $2 AND role <> 'owner'",
        )
        .bind(room_id.as_uuid())
        .bind(principal.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        if res.rows_affected() > 0 {
            sqlx::query(
                "UPDATE rooms SET member_count = GREATEST(member_count - 1, 0), updated_at = NOW() WHERE id = $1",
            )
            .bind(room_id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(|e| MessagingError::Other(e.into()))?;
        }
        tx.commit().await.map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn insert_post(&self, post: &RoomPost) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO room_posts
               (id, room_id, author_id, kind, content, media_id, is_edited, is_deleted, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(post.id.as_uuid())
        .bind(post.room_id.as_uuid())
        .bind(post.author_id.as_uuid())
        .bind(kind_str(post.kind))
        .bind(&post.content)
        .bind(post.media_id)
        .bind(post.is_edited)
        .bind(post.is_deleted)
        .bind(post.created_at)
        .bind(post.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| MessagingError::Other(e.into()))?;
        Ok(())
    }

    async fn list_posts(
        &self,
        room_id: RoomId,
        before: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<RoomPost>> {
        let limit = limit.clamp(1, 100);
        let rows: Vec<(Uuid, Uuid, Uuid, String, String, Option<Uuid>, bool, bool, chrono::DateTime<Utc>, chrono::DateTime<Utc>)> =
            if let Some(before) = before {
                sqlx::query_as(
                    r#"SELECT id, room_id, author_id, kind, content, media_id, is_edited, is_deleted, created_at, updated_at
                       FROM room_posts
                       WHERE room_id = $1 AND NOT is_deleted
                         AND created_at < (SELECT created_at FROM room_posts WHERE id = $2)
                       ORDER BY created_at DESC LIMIT $3"#,
                )
                .bind(room_id.as_uuid())
                .bind(before)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            } else {
                sqlx::query_as(
                    r#"SELECT id, room_id, author_id, kind, content, media_id, is_edited, is_deleted, created_at, updated_at
                       FROM room_posts
                       WHERE room_id = $1 AND NOT is_deleted
                       ORDER BY created_at DESC LIMIT $2"#,
                )
                .bind(room_id.as_uuid())
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            .map_err(|e| MessagingError::Other(e.into()))?;
        rows.into_iter()
            .map(|(a, b, c, d, e, f, g, h, i, j)| Self::map_post(a, b, c, d, e, f, g, h, i, j))
            .collect()
    }
}

pub struct RoomService {
    store: Arc<dyn RoomStore>,
}

impl RoomService {
    pub fn new(store: Arc<dyn RoomStore>) -> Self {
        Self { store }
    }

    pub async fn create(
        &self,
        created_by: PrincipalId,
        slug: &str,
        title: &str,
        description: Option<String>,
        visibility: RoomVisibility,
    ) -> Result<Room> {
        let slug = slugify(slug)?;
        let title = sanitize_text(title, MAX_TITLE)?;
        if title.is_empty() {
            return Err(MessagingError::InvalidArgument("title required".into()));
        }
        let description = match description {
            Some(d) => {
                let d = sanitize_text(&d, MAX_DESCRIPTION)?;
                if d.is_empty() { None } else { Some(d) }
            }
            None => None,
        };
        let now = Utc::now();
        let room = Room {
            id: RoomId::new(),
            slug,
            title,
            description,
            visibility,
            created_by,
            member_count: 1,
            is_archived: false,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_room(&room, created_by).await?;
        info!(slug = %room.slug, visibility = ?visibility, "room created");
        Ok(room)
    }

    pub async fn discover(&self, limit: i64, offset: i64) -> Result<Vec<Room>> {
        self.store.list_discoverable(limit, offset).await
    }

    pub async fn mine(&self, principal: PrincipalId) -> Result<Vec<Room>> {
        self.store.list_joined(principal).await
    }

    pub async fn get(&self, slug_or_id: &str) -> Result<Room> {
        if let Ok(id) = slug_or_id.parse::<RoomId>() {
            return self
                .store
                .get_by_id(id)
                .await?
                .ok_or_else(|| MessagingError::NotFound);
        }
        self.store
            .get_by_slug(slug_or_id)
            .await?
            .ok_or_else(|| MessagingError::NotFound)
    }

    /// Public rooms: any authenticated user may join.
    /// Private rooms: must already be a member (invite path later).
    pub async fn join(&self, slug: &str, principal: PrincipalId) -> Result<Room> {
        let room = self.get(slug).await?;
        if room.is_archived {
            return Err(MessagingError::InvalidArgument("room archived".into()));
        }
        match room.visibility {
            RoomVisibility::Public => {
                self.store.join(room.id, principal, "member").await?;
            }
            RoomVisibility::Private => {
                if !self.store.is_member(room.id, principal).await? {
                    return Err(MessagingError::Forbidden(
                        "private room — invite required".into(),
                    ));
                }
            }
        }
        self.get(&room.slug).await
    }

    pub async fn leave(&self, slug: &str, principal: PrincipalId) -> Result<()> {
        let room = self.get(slug).await?;
        // Store only deletes non-owner rows; surface a clear error if still member as owner
        self.store.leave(room.id, principal).await?;
        if self.store.is_member(room.id, principal).await? {
            return Err(MessagingError::Forbidden(
                "room owner cannot leave — transfer ownership first".into(),
            ));
        }
        Ok(())
    }

    /// Read feed: public rooms → any authed user; private → members only.
    pub async fn can_read(&self, room: &Room, principal: PrincipalId) -> Result<bool> {
        if matches!(room.visibility, RoomVisibility::Public) {
            return Ok(true);
        }
        self.store.is_member(room.id, principal).await
    }

    /// Post: must be a Roomie (member). Public join first if needed.
    pub async fn post(
        &self,
        slug: &str,
        author: PrincipalId,
        kind: RoomPostKind,
        content: String,
        media_id: Option<Uuid>,
    ) -> Result<RoomPost> {
        let room = self.get(slug).await?;
        if room.is_archived {
            return Err(MessagingError::InvalidArgument("room archived".into()));
        }
        if !self.store.is_member(room.id, author).await? {
            if matches!(room.visibility, RoomVisibility::Public) {
                self.store.join(room.id, author, "member").await?;
            } else {
                return Err(MessagingError::Forbidden("join the room first".into()));
            }
        }
        let content = sanitize_text(&content, MAX_POST_CONTENT)?;
        match kind {
            RoomPostKind::Text => {
                if content.is_empty() {
                    return Err(MessagingError::InvalidArgument("content required".into()));
                }
            }
            RoomPostKind::VoiceNote | RoomPostKind::VideoClip | RoomPostKind::Attachment => {
                if media_id.is_none() {
                    return Err(MessagingError::InvalidArgument(
                        "media_id required for this post kind".into(),
                    ));
                }
            }
        }
        let now = Utc::now();
        let post = RoomPost {
            id: RoomPostId::new(),
            room_id: room.id,
            author_id: author,
            kind,
            content,
            media_id,
            is_edited: false,
            is_deleted: false,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_post(&post).await?;
        // Fanout: Redis today; Apache Iggy when volume demands it
        Ok(post)
    }

    pub async fn feed(
        &self,
        slug: &str,
        principal: PrincipalId,
        before: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<RoomPost>> {
        let room = self.get(slug).await?;
        if !self.can_read(&room, principal).await? {
            return Err(MessagingError::Forbidden("cannot read this room".into()));
        }
        self.store.list_posts(room.id, before, limit).await
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[test]
    fn slugify_accepts_community_handles() {
        assert_eq!(slugify("#Rust").unwrap(), "rust");
        assert_eq!(slugify("Turing-Tarpit").unwrap(), "turing-tarpit");
        assert_eq!(slugify("  go_lang  ").unwrap(), "go_lang");
    }

    #[test]
    fn slugify_rejects_reserved_and_bad() {
        assert!(slugify("api").is_err());
        assert!(slugify("mine").is_err());
        assert!(slugify("a").is_err()); // too short
        assert!(slugify("12345").is_err());
        assert!(slugify("").is_err());
        assert!(slugify(&"x".repeat(65)).is_err());
    }

    #[test]
    fn slugify_collapses_dashes() {
        assert_eq!(slugify("foo---bar").unwrap(), "foo-bar");
    }

    #[test]
    fn sanitize_strips_controls_and_limits() {
        assert_eq!(sanitize_text("  hello \0 world  ", 100).unwrap(), "hello  world");
        assert!(sanitize_text(&"a".repeat(MAX_POST_CONTENT + 1), MAX_POST_CONTENT).is_err());
    }

    #[derive(Default)]
    struct MemStore {
        rooms: Mutex<HashMap<String, Room>>,
        by_id: Mutex<HashMap<Uuid, String>>,
        members: Mutex<HashMap<(Uuid, Uuid), String>>,
        posts: Mutex<Vec<RoomPost>>,
    }

    #[async_trait]
    impl RoomStore for MemStore {
        async fn insert_room(&self, room: &Room, owner: PrincipalId) -> Result<()> {
            let mut g = self.rooms.lock().unwrap();
            if g.contains_key(&room.slug) {
                return Err(MessagingError::Conflict("slug already taken".into()));
            }
            g.insert(room.slug.clone(), room.clone());
            self.by_id
                .lock()
                .unwrap()
                .insert(*room.id.as_uuid(), room.slug.clone());
            self.members
                .lock()
                .unwrap()
                .insert((*room.id.as_uuid(), *owner.as_uuid()), "owner".into());
            Ok(())
        }
        async fn get_by_slug(&self, slug: &str) -> Result<Option<Room>> {
            Ok(self.rooms.lock().unwrap().get(slug).cloned())
        }
        async fn get_by_id(&self, id: RoomId) -> Result<Option<Room>> {
            let slug = self.by_id.lock().unwrap().get(id.as_uuid()).cloned();
            Ok(slug.and_then(|s| self.rooms.lock().unwrap().get(&s).cloned()))
        }
        async fn list_discoverable(&self, limit: i64, _offset: i64) -> Result<Vec<Room>> {
            let mut v: Vec<_> = self
                .rooms
                .lock()
                .unwrap()
                .values()
                .filter(|r| matches!(r.visibility, RoomVisibility::Public) && !r.is_archived)
                .cloned()
                .collect();
            v.truncate(limit as usize);
            Ok(v)
        }
        async fn list_joined(&self, principal: PrincipalId) -> Result<Vec<Room>> {
            let mem = self.members.lock().unwrap();
            let rooms = self.rooms.lock().unwrap();
            Ok(rooms
                .values()
                .filter(|r| mem.contains_key(&(*r.id.as_uuid(), *principal.as_uuid())))
                .cloned()
                .collect())
        }
        async fn is_member(&self, room_id: RoomId, principal: PrincipalId) -> Result<bool> {
            Ok(self
                .members
                .lock()
                .unwrap()
                .contains_key(&(*room_id.as_uuid(), *principal.as_uuid())))
        }
        async fn join(&self, room_id: RoomId, principal: PrincipalId, role: &str) -> Result<()> {
            let mut mem = self.members.lock().unwrap();
            let key = (*room_id.as_uuid(), *principal.as_uuid());
            if !mem.contains_key(&key) {
                mem.insert(key, role.into());
                if let Some(slug) = self.by_id.lock().unwrap().get(room_id.as_uuid()).cloned() {
                    if let Some(r) = self.rooms.lock().unwrap().get_mut(&slug) {
                        r.member_count += 1;
                    }
                }
            }
            Ok(())
        }
        async fn leave(&self, room_id: RoomId, principal: PrincipalId) -> Result<()> {
            let mut mem = self.members.lock().unwrap();
            let key = (*room_id.as_uuid(), *principal.as_uuid());
            if mem.get(&key).map(|r| r.as_str()) == Some("owner") {
                return Ok(()); // leave no-op for owner; service detects
            }
            if mem.remove(&key).is_some() {
                if let Some(slug) = self.by_id.lock().unwrap().get(room_id.as_uuid()).cloned() {
                    if let Some(r) = self.rooms.lock().unwrap().get_mut(&slug) {
                        r.member_count = r.member_count.saturating_sub(1);
                    }
                }
            }
            Ok(())
        }
        async fn insert_post(&self, post: &RoomPost) -> Result<()> {
            self.posts.lock().unwrap().push(post.clone());
            Ok(())
        }
        async fn list_posts(
            &self,
            room_id: RoomId,
            _before: Option<Uuid>,
            limit: i64,
        ) -> Result<Vec<RoomPost>> {
            let mut v: Vec<_> = self
                .posts
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.room_id == room_id && !p.is_deleted)
                .cloned()
                .collect();
            v.truncate(limit as usize);
            Ok(v)
        }
    }

    fn pid() -> PrincipalId {
        PrincipalId::from_uuid(Uuid::now_v7())
    }

    #[tokio::test]
    async fn public_room_join_post_feed() {
        let svc = RoomService::new(Arc::new(MemStore::default()));
        let owner = pid();
        let room = svc
            .create(owner, "rust", "Rust", Some("lang".into()), RoomVisibility::Public)
            .await
            .unwrap();
        assert_eq!(room.slug, "rust");
        assert_eq!(room.member_count, 1);

        let other = pid();
        let joined = svc.join("rust", other).await.unwrap();
        assert!(joined.member_count >= 1);

        let post = svc
            .post("rust", other, RoomPostKind::Text, "hello roomies".into(), None)
            .await
            .unwrap();
        assert_eq!(post.content, "hello roomies");

        let feed = svc.feed("rust", other, None, 10).await.unwrap();
        assert_eq!(feed.len(), 1);
    }

    #[tokio::test]
    async fn private_room_blocks_stranger_read_and_join() {
        let svc = RoomService::new(Arc::new(MemStore::default()));
        let owner = pid();
        svc.create(owner, "secret-cabal", "Cabal", None, RoomVisibility::Private)
            .await
            .unwrap();
        let stranger = pid();
        assert!(svc.join("secret-cabal", stranger).await.is_err());
        assert!(svc.feed("secret-cabal", stranger, None, 10).await.is_err());
        // owner can read
        assert!(svc.feed("secret-cabal", owner, None, 10).await.is_ok());
    }

    #[tokio::test]
    async fn discover_only_lists_public() {
        let svc = RoomService::new(Arc::new(MemStore::default()));
        let owner = pid();
        svc.create(owner, "rust", "Rust", None, RoomVisibility::Public)
            .await
            .unwrap();
        svc.create(owner, "hidden", "Hidden", None, RoomVisibility::Private)
            .await
            .unwrap();
        let d = svc.discover(50, 0).await.unwrap();
        assert!(d.iter().any(|r| r.slug == "rust"));
        assert!(!d.iter().any(|r| r.slug == "hidden"));
    }

    #[tokio::test]
    async fn owner_cannot_leave() {
        let svc = RoomService::new(Arc::new(MemStore::default()));
        let owner = pid();
        svc.create(owner, "go", "Go", None, RoomVisibility::Public)
            .await
            .unwrap();
        let err = svc.leave("go", owner).await.unwrap_err();
        assert!(matches!(err, MessagingError::Forbidden(_)));
    }

    #[tokio::test]
    async fn media_kinds_require_media_id() {
        let svc = RoomService::new(Arc::new(MemStore::default()));
        let owner = pid();
        svc.create(owner, "ai", "AI", None, RoomVisibility::Public)
            .await
            .unwrap();
        assert!(svc
            .post("ai", owner, RoomPostKind::VoiceNote, "clip".into(), None)
            .await
            .is_err());
        let media = Uuid::now_v7();
        let p = svc
            .post(
                "ai",
                owner,
                RoomPostKind::VoiceNote,
                "caption".into(),
                Some(media),
            )
            .await
            .unwrap();
        assert_eq!(p.media_id, Some(media));
    }

    #[tokio::test]
    async fn duplicate_slug_conflicts() {
        let svc = RoomService::new(Arc::new(MemStore::default()));
        let owner = pid();
        svc.create(owner, "rust", "Rust", None, RoomVisibility::Public)
            .await
            .unwrap();
        let err = svc
            .create(owner, "Rust", "Other", None, RoomVisibility::Public)
            .await
            .unwrap_err();
        assert!(matches!(err, MessagingError::Conflict(_)));
    }
}
