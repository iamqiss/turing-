//! Turing Tarpit server entrypoint.
//!
//! - HTTP  :8080  – health, REST APIs, Git Smart HTTP
//! - gRPC  :50051 – Auth + Messaging (HTTP/2) + gRPC-Web (HTTP/1.1)

mod authz;
mod bootstrap;
mod routes;
mod state;
mod push_observer;
mod agent_adapters;

use std::net::SocketAddr;

use tracing::info;
use tracing_subscriber::EnvFilter;
use tt_api::pb::auth::v1::auth_service_server::AuthServiceServer;
use tt_api::pb::messaging::v1::messaging_service_server::MessagingServiceServer;
use tt_api::{AuthServiceImpl, MessagingServiceImpl};
use tt_config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,tt_server=debug,tt_auth=debug,tt_api=debug,tt_messaging=debug")
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();

    info!("Turing Tarpit starting");

    let config = Config::load()?;
    info!(
        host = %config.host,
        port = config.port,
        grpc_port = config.grpc_port,
        "config loaded"
    );

    let state = bootstrap::build_state(config.clone()).await?;

    let auth_svc = AuthServiceImpl::new(
        state.webauthn.clone(),
        state.sessions.clone(),
        state.agent_tokens.clone(),
        state.principal_resolver.clone(),
        state.passphrases.clone(),
    );

    let messaging_svc = MessagingServiceImpl::new(
        state.messaging.clone(),
        state.principal_resolver.clone(),
        state.realtime.clone(),
    );

    let grpc_addr: SocketAddr = format!("{}:{}", config.host, config.grpc_port).parse()?;

    let grpc_server = tonic::transport::Server::builder()
        .accept_http1(true)
        .add_service(tonic_web::enable(AuthServiceServer::new(auth_svc)))
        .add_service(tonic_web::enable(MessagingServiceServer::new(messaging_svc)))
        .serve_with_shutdown(grpc_addr, shutdown_signal());

    let http_app = routes::router(state);
    let http_addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    let http_listener = tokio::net::TcpListener::bind(http_addr).await?;

    info!(%http_addr, "HTTP listening (health + REST + git)");
    info!(%grpc_addr, "gRPC + gRPC-Web listening (auth + messaging)");

    let http_server = axum::serve(http_listener, http_app).with_graceful_shutdown(shutdown_signal());

    tokio::select! {
        res = http_server => {
            if let Err(e) = res {
                tracing::error!(error = %e, "HTTP server error");
            }
        }
        res = grpc_server => {
            if let Err(e) = res {
                tracing::error!(error = %e, "gRPC server error");
            }
        }
    }

    info!("shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sig = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        sig.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("received Ctrl+C"),
        _ = terminate => info!("received SIGTERM"),
    }
}
