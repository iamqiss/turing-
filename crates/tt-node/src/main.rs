//! tt-node — bare-metal agent for Turing Tarpit Cloud.
//!
//! Drivers: Firecracker (C1b), Cloud Hypervisor (stub), Kata (stub).

mod drivers;
mod deploy;
mod netfilter;
mod guest_net;
mod bridge;
mod cidata;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use serde::Serialize;
use tracing::info;

use drivers::cloud_hypervisor::CloudHypervisorDriver;
use drivers::firecracker::FirecrackerDriver;
use drivers::kata::KataDriver;
use drivers::{dispatch, DriverRequest};

#[derive(Parser, Debug)]
#[command(name = "tt-node", about = "Turing Tarpit compute agent")]
struct Args {
    #[arg(long, env = "TT_NODE_ID")]
    node_id: Option<String>,

    #[arg(long, env = "TT_NODE_BIND", default_value = "0.0.0.0:9090")]
    bind: String,

    #[arg(long, env = "TT_CONTROL_PLANE", default_value = "http://127.0.0.1:8080")]
    control_plane: String,

    #[arg(long, env = "TT_REGION", default_value = "eu-central")]
    region: String,

    /// Instance + image storage root
    #[arg(long, env = "TT_NODE_DATA", default_value = "/var/lib/tt-node")]
    data_dir: PathBuf,
}

#[derive(Clone)]
struct AppState {
    node_id: Option<String>,
    region: String,
    data_dir: PathBuf,
    firecracker: FirecrackerDriver,
    cloud_hypervisor: CloudHypervisorDriver,
    kata: KataDriver,
}

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
    node_id: Option<String>,
    region: String,
    data_dir: String,
    drivers: Vec<DriverHealth>,
}

#[derive(Debug, Serialize)]
struct DriverHealth {
    name: &'static str,
    ready: bool,
    detail: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    tokio::fs::create_dir_all(args.data_dir.join("instances")).await.ok();
    tokio::fs::create_dir_all(args.data_dir.join("images")).await.ok();

    let firecracker = FirecrackerDriver::new(args.data_dir.clone());
    let cloud_hypervisor = CloudHypervisorDriver::new(args.data_dir.clone());
    let kata = KataDriver;

    info!(
        region = %args.region,
        bind = %args.bind,
        data = %args.data_dir.display(),
        "tt-node starting (Firecracker C1b)"
    );

    let state = AppState {
        node_id: args.node_id,
        region: args.region,
        data_dir: args.data_dir.clone(),
        firecracker,
        cloud_hypervisor,
        kata,
    };

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/v1/command", post(command))
        .with_state(Arc::new(state));

    let addr: SocketAddr = args.bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "tt-node listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(st): State<Arc<AppState>>) -> Json<Health> {
    let fc_bin = std::env::var("TT_FIRECRACKER_BIN").unwrap_or_else(|_| "firecracker".into());
    let fc_ready = which_cmd(&fc_bin);
    Json(Health {
        status: "ok",
        node_id: st.node_id.clone(),
        region: st.region.clone(),
        data_dir: st.data_dir.display().to_string(),
        drivers: vec![
            DriverHealth {
                name: "firecracker",
                ready: fc_ready,
                detail: if fc_ready {
                    "binary found".into()
                } else {
                    format!("{fc_bin} not on PATH — dry-run mode")
                },
            },
            DriverHealth {
                name: "cloud_hypervisor",
                ready: false,
                detail: "stub (C3)".into(),
            },
            DriverHealth {
                name: "kata",
                ready: false,
                detail: "stub".into(),
            },
        ],
    })
}

async fn command(
    State(st): State<Arc<AppState>>,
    Json(req): Json<DriverRequest>,
) -> impl IntoResponse {
    info!(
        instance = %req.instance_id,
        driver = %req.driver,
        action = %req.action,
        "driver command"
    );
    match dispatch(&st.firecracker, &st.cloud_hypervisor, &st.kata, req).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "accepted": false,
                "error": e.to_string(),
            })),
        )
            .into_response(),
    }
}

fn which_cmd(bin: &str) -> bool {
    let p = std::path::Path::new(bin);
    if p.is_absolute() {
        return p.exists();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).exists()))
        .unwrap_or(false)
}
