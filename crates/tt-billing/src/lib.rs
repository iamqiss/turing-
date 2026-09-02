//! Turing Tarpit billing — seats + usage, never conflated.
//!
//! Seats → collaboration (messaging, git, rooms, issues).
//! Usage → compute, transfer, storage, managed data (Postgres / EmberDB / RustFS / Greptime).

mod catalog;
mod domain;
mod error;
mod service;
mod store;

pub use catalog::{Catalog, Meter, Micros, Plan, PlanId};
pub use domain::*;
pub use error::{BillingError, Result};
pub use service::BillingService;
pub use store::{BillingStore, PostgresBillingStore};
