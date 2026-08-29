//! Realtime fanout abstraction.
//!
//! Current backends: in-memory, Redis (stub).
//! Future: Apache Iggy.

pub mod publisher;
pub mod subscriber;
pub mod redis_backend;
pub mod iggy_backend;
pub mod in_memory;

pub use publisher::EventPublisher;
pub use subscriber::{EventStream, EventSubscriber};
pub use in_memory::InMemoryRealtime;
