//! Storage & contribution analytics.
//!
//! Dual-write contribution events to Postgres (source of truth for recent queries)
//! and ClickHouse when available (long-window analytics).

pub mod error;
pub mod clickhouse;
pub mod events;
pub mod profile;

pub use error::{Result, StorageError};
pub use events::{ContributionStore, PostgresContributionStore};
pub use clickhouse::ClickHouseClient;
pub use profile::{ProfileService, PostgresProfileStore, ProfileStore};
