pub mod api;
pub mod lila;
pub mod model;
pub mod db;

use clap::Clap;
use axum::{handler::get, Router};
use std::net::SocketAddr;
use std::sync::Arc;
use crate::db::Database;

#[derive(Clap)]
struct Opt {
    #[clap(long = "bind", default_value = "127.0.0.1:9000")]
    bind: SocketAddr,
}

#[tokio::main]
async fn main() {
    let opt = Opt::parse();

    let db = Arc::new(Database::open());

    let app = Router::new().route("/", get(|| async { dbg!(db); "Hello world!" }));

    axum::Server::bind(&opt.bind)
        .serve(app.into_make_service())
        .await
        .expect("bind");
}
