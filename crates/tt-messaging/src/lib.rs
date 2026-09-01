//! Messaging: channels, messages, threads, reactions, workspaces.

pub mod error;
pub mod store;
pub mod service;
pub mod http;
pub mod workspace;
mod org;
mod rooms;
mod rooms_http;

pub use error::{MessagingError, Result};
pub use store::{MessageStore, PostgresMessageStore};
pub use service::MessagingService;
pub use http::{router as messaging_http_router, MessagingHttpState};
pub use workspace::{PostgresWorkspaceStore, WorkspaceService, WorkspaceStore};
pub use org::{OrgService, OrgStore, PostgresOrgStore};
pub use rooms::{PostgresRoomStore, RoomService, RoomStore};
pub use rooms_http::{router as rooms_http_router, RoomsHttpState};
