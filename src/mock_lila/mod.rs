//! A stand-in for lila's games export, serving games of a fixture prepared
//! from the parquet, for tests and benchmarks of the personal explorer.

pub mod fixture;
pub mod prepare;
pub mod serve;
pub mod synth;

pub use fixture::{Fixture, FixtureGame, FixtureMeta, FixturePlayer, FixtureWriter};
pub use serve::{Faults, MockConfig, MockLila, MockStats, Noise};
