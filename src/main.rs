pub mod api;
pub mod db;
pub mod lila;
pub mod model;

use crate::db::Database;
use axum::{extract::Extension, handler::get, AddExtensionLayer, Router};
use clap::Clap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clap)]
struct Opt {
    #[clap(long = "bind", default_value = "127.0.0.1:9000")]
    bind: SocketAddr,
    #[clap(long = "db", default_value = "_db")]
    db: PathBuf,
}

#[tokio::main]
async fn main() {
    let opt = Opt::parse();

    let db = Arc::new(Database::open(opt.db).expect("db"));

    let app = Router::new()
        .route("/", get(hello_world))
        .layer(AddExtensionLayer::new(db));

    axum::Server::bind(&opt.bind)
        .serve(app.into_make_service())
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.expect("wait for ctrl-c");
        })
        .await
        .expect("bind");
}

async fn hello_world(Extension(db): Extension<Arc<Database>>) -> String {
    db.inner.put("hello", "world").expect("put");
    String::from_utf8(db.inner.get("hello").expect("get").expect("present")).expect("utf-8")
}
