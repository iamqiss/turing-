//! Realtime fanout abstraction.
//!
//! Backends:
//! - **memory** — in-process broadcast (default, always available)
//! - **redis** — stub / future
//! - **iggy** — Apache Iggy HTTP dual-write (+ local mirror)

pub mod publisher;
pub mod subscriber;
pub mod redis_backend;
pub mod iggy_backend;
pub mod in_memory;

pub use publisher::EventPublisher;
pub use subscriber::{EventStream, EventSubscriber};
pub use in_memory::InMemoryRealtime;
pub use iggy_backend::{IggyConfig, IggyRealtime};
