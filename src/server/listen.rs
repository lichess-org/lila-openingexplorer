use std::{io, net::SocketAddr, time::Duration};

use axum::{
    Router,
    http::{Method, header},
    response::{IntoResponse, Response},
};
use tokio::{
    net::TcpListener,
    signal::unix::{SignalKind, signal},
};
use tokio_util::sync::CancellationToken;
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};

use crate::util::{Exposition, METRICS_CONTENT_TYPE};

pub fn metrics_response(out: Exposition) -> Response {
    ([(header::CONTENT_TYPE, METRICS_CONTENT_TYPE)], out.finish()).into_response()
}

pub fn cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_credentials(true)
        .allow_methods([Method::GET])
        .allow_headers(AllowHeaders::mirror_request())
        .max_age(Duration::from_secs(24 * 60 * 60))
}

async fn shutdown_signal() {
    let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => log::info!("received SIGINT, shutting down"),
        _ = sigterm.recv() => log::info!("received SIGTERM, shutting down"),
    }
}

pub async fn serve_until_signal(
    bind: SocketAddr,
    app: Router,
    admin: Option<(SocketAddr, Router)>,
) -> io::Result<()> {
    let public = TcpListener::bind(bind).await?;
    let admin = match admin {
        Some((addr, router)) => Some((TcpListener::bind(addr).await?, addr, router)),
        None => None,
    };

    let shutdown = CancellationToken::new();

    log::info!("listening on {bind}");
    let app = tokio::spawn(
        axum::serve(public, app)
            .with_graceful_shutdown(shutdown.clone().cancelled_owned())
            .into_future(),
    );

    let admin = match admin {
        Some((listener, addr, router)) => {
            log::info!("administration and monitoring on {addr}");
            Some(tokio::spawn(
                axum::serve(listener, router)
                    .with_graceful_shutdown(shutdown.clone().cancelled_owned())
                    .into_future(),
            ))
        }
        None => {
            log::info!("administration and monitoring disabled (--admin-bind)");
            None
        }
    };

    shutdown_signal().await;

    shutdown.cancel();
    app.await??;
    if let Some(admin) = admin {
        admin.await??;
    }

    Ok(())
}
