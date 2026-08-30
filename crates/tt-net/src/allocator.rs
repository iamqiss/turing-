//! NAT port-range map + IPv6 /64 assignment.
//!
//! CloudRat model: ports < 10000 reserved for host; each shared IPv4 serves
//! ~10 customers in ~5553-port blocks.

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;
use tracing::info;
use tt_core::domain::{IpKind, RegionCode};
use tt_core::ids::{InstanceId, SubnetId};
use uuid::Uuid;

use crate::error::{NetError, Result};

const HOST_PORT_RESERVE: u16 = 10_000;
const PORTS_PER_CUSTOMER: u16 = 5_553;
const CUSTOMERS_PER_IP: u16 = 10;

#[derive(Debug, Clone)]
pub struct NatAssignment {
    pub public_ip: String,
    pub port_start: u16,
    pub port_end: u16,
    pub ipv6: Option<String>,
}

#[async_trait]
pub trait IpAllocator: Send + Sync {
    async fn allocate_nat(
        &self,
        region: RegionCode,
        instance_id: InstanceId,
    ) -> Result<NatAssignment>;
    async fn release(&self, instance_id: InstanceId) -> Result<()>;
    async fn register_shared_ip(
        &self,
        region: RegionCode,
        cidr_or_ip: &str,
        node_id: Option<Uuid>,
    ) -> Result<SubnetId>;
}

pub struct PostgresIpAllocator {
    pool: PgPool,
}

impl PostgresIpAllocator {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl IpAllocator for PostgresIpAllocator {
    async fn register_shared_ip(
        &self,
        region: RegionCode,
        cidr_or_ip: &str,
        node_id: Option<Uuid>,
    ) -> Result<SubnetId> {
        let id = SubnetId::new();
        sqlx::query(
            r#"
            INSERT INTO subnets (id, region, cidr, kind, node_id, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(id.as_uuid())
        .bind(region.as_str())
        .bind(cidr_or_ip)
        .bind(IpKind::SharedNatV4.as_str())
        .bind(node_id)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(|e| NetError::Other(e.into()))?;
        info!(%cidr_or_ip, region = region.as_str(), "shared NAT IP registered");
        Ok(id)
    }

    async fn allocate_nat(
        &self,
        region: RegionCode,
        instance_id: InstanceId,
    ) -> Result<NatAssignment> {
        let subnets: Vec<(Uuid, String)> = sqlx::query_as(
            r#"
            SELECT id, cidr FROM subnets
            WHERE region = $1 AND kind = 'shared_nat_v4'
            ORDER BY created_at
            "#,
        )
        .bind(region.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| NetError::Other(e.into()))?;

        if subnets.is_empty() {
            return Err(NetError::Exhausted(format!(
                "no shared NAT subnets in {}",
                region.as_str()
            )));
        }

        for (subnet_id, public_ip) in subnets {
            let (used,): (i64,) = sqlx::query_as(
                "SELECT COUNT(*)::bigint FROM ip_allocations WHERE subnet_id = $1",
            )
            .bind(subnet_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| NetError::Other(e.into()))?;

            if used >= CUSTOMERS_PER_IP as i64 {
                continue;
            }

            let slot = used as u16;
            let port_start = HOST_PORT_RESERVE + slot * PORTS_PER_CUSTOMER;
            let port_end = port_start + PORTS_PER_CUSTOMER - 1;

            let alloc_id = Uuid::now_v7();
            sqlx::query(
                r#"
                INSERT INTO ip_allocations (
                    id, subnet_id, instance_id, address, port_start, port_end, kind, created_at
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
                "#,
            )
            .bind(alloc_id)
            .bind(subnet_id)
            .bind(instance_id.as_uuid())
            .bind(&public_ip)
            .bind(port_start as i32)
            .bind(port_end as i32)
            .bind(IpKind::SharedNatV4.as_str())
            .bind(Utc::now())
            .execute(&self.pool)
            .await
            .map_err(|e| NetError::Other(e.into()))?;

            let ipv6 = Some(format!("2001:db8:tt:{:x}::1", alloc_id.as_u128() & 0xffff));

            return Ok(NatAssignment {
                public_ip,
                port_start,
                port_end,
                ipv6,
            });
        }

        Err(NetError::Exhausted(format!(
            "shared NAT capacity full in {}",
            region.as_str()
        )))
    }

    async fn release(&self, instance_id: InstanceId) -> Result<()> {
        sqlx::query("DELETE FROM ip_allocations WHERE instance_id = $1")
            .bind(instance_id.as_uuid())
            .execute(&self.pool)
            .await
            .map_err(|e| NetError::Other(e.into()))?;
        Ok(())
    }
}
