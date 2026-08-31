//! Storage & contribution analytics.
//!
//! Dual-write contribution events to Postgres (source of truth for recent queries)
//! and ClickHouse when available (long-window analytics).

pub mod error;
pub mod clickhouse;
pub mod events;
pub mod lifecycle;
pub mod profile;
pub mod object_store;
pub mod media;

pub use error::{Result, StorageError};
pub use events::{ContributionStore, PostgresContributionStore};
pub use lifecycle::{LifecycleStore, PostgresLifecycleStore, new_lifecycle};
pub use clickhouse::ClickHouseClient;
pub use profile::{ProfileService, PostgresProfileStore, ProfileStore};
pub use object_store::{LocalObjectStore, ObjectStore};
pub use media::{MediaObject, MediaService};
