//! IP / subnet allocation (CloudRat NAT + IPv6-default model).

mod error;
mod allocator;

pub use error::{NetError, Result};
pub use allocator::{IpAllocator, NatAssignment, PostgresIpAllocator};
