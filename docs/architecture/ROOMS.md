# Rooms

Community spaces on Turing Tarpit. Members are **Roomies**.

Live multi-party video is **intentionally postponed**. Rooms today are feeds with text, voice notes, short clips, and attachments — public & discoverable for topics like `#Rust`, `#Go`, `#AI`.

## Model

| Field | Notes |
|-------|--------|
| `slug` | Handle without `#` (`rust`, `turing-tarpit`) |
| `visibility` | `public` (discoverable, open join) or `private` |
| Posts | `text` · `voice_note` · `video_clip` · `attachment` |
| Media | Reuses media pipeline (`media_id` → `/api/media/:id`) |

## Permissions

| Action | Public room | Private room |
|--------|-------------|--------------|
| Discover in catalog | yes | no |
| Read feed | any authenticated user | members only |
| Join | any authenticated user | invite (later) |
| Post | auto-join then post | members only |

## API

```http
GET    /api/rooms                 # discover public
POST   /api/rooms                 # create { slug, title, description?, visibility? }
GET    /api/rooms/mine
GET    /api/rooms/:slug
POST   /api/rooms/:slug/join
POST   /api/rooms/:slug/leave
GET    /api/rooms/:slug/posts
POST   /api/rooms/:slug/posts     # { kind, content?, media_id? }
```

Voice / clip: upload media first, then post with `kind` + `media_id`.

## Fanout

Redis / in-process today. **Apache Iggy** when room traffic is the bottleneck — same event shape, swap publisher.

## Schema

Migration `022_rooms.sql` — `rooms`, `room_members`, `room_posts`.


## Hardening

- Slug: 2–64 chars, `[a-z0-9_-]`, not purely numeric, reserved names blocked (`api`, `mine`, `admin`, …)
- Title ≤ 120, description ≤ 2000, post body ≤ 8000 characters
- Control characters stripped from text (newline/tab kept)
- Public: open join + read for authenticated users; post auto-joins
- Private: no discover listing; join/read require membership
- Room owner cannot leave without ownership transfer
- Voice/clip/attachment posts require `media_id`
- Fanout still Redis-era; Iggy when load demands

## Tests

```bash
cargo test -p tt-messaging rooms::
cargo test -p tt-secrets
cargo test -p tt-storage object_store::
```
