pub mod aggregate;
pub mod bulk;
pub mod db;
pub mod explore;
pub mod game;
pub mod indexer;
pub mod keys;
pub mod lila;
pub mod policy;
pub mod query;
pub mod queue;
pub mod server;
pub mod status;
#[cfg(test)]
mod tests;
pub mod time;
mod user;

pub use db::{PlayerDb, PlayerDbConfig, PlayerDbError};

/// Plies of each game that are indexed. Must not change on an existing
/// database.
pub const MAX_PLY: usize = 50;

pub use user::{InvalidUserName, UserHash, UserHasher, UserId};
