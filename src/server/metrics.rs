use crate::{
    server::{app::Kind, query::Source},
    util::{Counter, Exposition, Histogram},
};

/// Valid requests by known source.
#[derive(Default)]
pub struct SourceCounters([Counter; Source::ALL.len()]);

impl SourceCounters {
    pub fn inc(&self, source: Option<Source>) {
        if let Some(source) = source {
            self.0[source as usize].inc();
        }
    }

    pub fn write(&self, out: &mut Exposition) {
        out.labeled_counter(
            "requests_by_source_total",
            "Valid requests by source query parameter, if it is a known one.",
            "source",
            Source::ALL.map(|source| (source.as_str(), self.0[source as usize].get())),
        );
    }
}

/// Request metrics for `/metrics`.
#[derive(Default)]
pub struct Metrics {
    /// Valid explorer requests, answered from the response cache or
    /// computed.
    pub requests: Counter,
    pub sources: SourceCounters,
    /// Explorer requests that computed their response, once for concurrent
    /// identical requests.
    pub computed: Counter,
    pub bad_requests: Counter,
    /// Positions found in the positions column family.
    pub positions_hits: Counter,
    /// Positions validated from a cold candidate.
    pub cold_hits: Counter,
    /// Positions found in neither column family.
    pub misses: Counter,
    /// Cold candidates loaded and replayed.
    pub cold_games_loaded: Counter,
    /// Games loaded for the moves, top and recent games of responses.
    pub response_games_loaded: Counter,
    /// Loaded games that were not in the block cache, fetched with
    /// --parallel-fetch.
    pub parallel_fetch_games: Counter,
    /// Lichess only: loaded games hidden because a player is blacklisted.
    pub blacklisted_games: Counter,
    /// Masters only.
    pub pgn_requests: Counter,
    /// Computed responses: time spent computing on the blocking pool.
    pub compute: Histogram,
    /// Successful explorer requests that computed their response: time
    /// from receiving the request to the response (including queueing for
    /// the blocking pool).
    pub request_computed: Histogram,
    /// Successful explorer requests that got their response from the cache,
    /// or from a concurrent identical request that computed it.
    pub request_not_computed: Histogram,
}

impl Metrics {
    /// The metrics relevant for the served database.
    pub fn write(&self, kind: Kind, out: &mut Exposition) {
        out.counter(
            "requests_total",
            "Valid explorer requests, answered from the response cache or computed.",
            self.requests.get(),
        );
        self.sources.write(out);
        out.counter(
            "responses_computed_total",
            "Explorer requests that computed their response (response cache misses), once for concurrent identical requests.",
            self.computed.get(),
        );
        out.counter(
            "bad_requests_total",
            "Rejected explorer requests.",
            self.bad_requests.get(),
        );
        out.labeled_counter(
            "position_lookups_total",
            "Position lookups by result: found in the positions column family, validated from a cold candidate, or not found.",
            "result",
            [
                ("positions", self.positions_hits.get()),
                ("cold", self.cold_hits.get()),
                ("miss", self.misses.get()),
            ],
        );
        out.labeled_counter(
            "games_loaded_total",
            "Games loaded, by purpose: cold candidates to replay, or moves, top and recent games of responses.",
            "purpose",
            [
                ("cold_candidate", self.cold_games_loaded.get()),
                ("response", self.response_games_loaded.get()),
            ],
        );
        out.counter(
            "parallel_fetch_games_total",
            "Loaded games that were not in the block cache, fetched in parallel (--parallel-fetch).",
            self.parallel_fetch_games.get(),
        );
        if kind == Kind::Lichess {
            out.counter(
                "blacklisted_games_total",
                "Loaded games hidden because a player is blacklisted.",
                self.blacklisted_games.get(),
            );
        }
        if kind == Kind::Masters {
            out.counter(
                "pgn_requests_total",
                "Requests for the PGN of a game.",
                self.pgn_requests.get(),
            );
        }
        out.histogram(
            "compute_duration_seconds",
            "Computed responses: time spent computing on the blocking pool.",
            &self.compute,
        );
        out.histogram_family(
            "request_duration_seconds",
            "Successful explorer requests: time from receiving the request to the response, by whether the request computed it (including queueing for the blocking pool), or got it from the cache or a concurrent identical request.",
            [
                (&[("computed", "true")][..], &self.request_computed),
                (&[("computed", "false")][..], &self.request_not_computed),
            ],
        );
    }
}
