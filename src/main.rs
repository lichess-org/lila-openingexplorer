pub mod api;
pub mod db;
pub mod lila;
pub mod model;

use crate::{
    api::{PersonalQuery, PersonalResponse},
    db::{Database, IndexerStub},
};
use axum::{
    extract::{Extension, Query},
    handler::get,
    response::Json,
    AddExtensionLayer, Router,
};
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

    let (indexer, join_handle) = IndexerStub::spawn(db.clone());

    let app = Router::new()
        .route("/personal", get(personal))
        .layer(AddExtensionLayer::new(db));

    axum::Server::bind(&opt.bind)
        .serve(app.into_make_service())
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.expect("wait for ctrl-c");
        })
        .await
        .expect("bind");

    join_handle.await.expect("indexer");
}

async fn personal(
    Extension(db): Extension<Arc<Database>>,
    Query(query): Query<PersonalQuery>,
) -> Json<PersonalResponse> {
    Json(PersonalResponse {})
}
