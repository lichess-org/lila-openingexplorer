use std::{
    cmp::{Ordering, Reverse, min},
    collections::{BTreeMap, BinaryHeap},
};

use partial_sort::PartialSort;
use shakmaty::packed::PackedUciMove;

use crate::{
    packed::{EntryView, GameIndex, GroupHeader, Month, RatingGroup, Speed, SubStats},
    server::query::{LichessFilter, Limits},
};

/// Most games referenced by a lichess response (top and recent games).
pub const MAX_LICHESS_GAMES: usize = 8;
/// Most top games of a lichess response.
pub const MAX_TOP_GAMES: usize = 4;
/// Most top games of a masters response.
pub const MAX_MASTERS_GAMES: usize = 15;
/// Masters: most recent games of the highest rating groups that are loaded
/// and ranked by their actual ratings, for the top games.
pub const MASTERS_TOP_CANDIDATES: usize = 128;

/// Statistics of a move, aggregated over the matching cells.
#[derive(Debug, Clone)]
pub struct PreparedMove {
    pub m: PackedUciMove,
    pub stats: SubStats,
    /// The only game, if the move was played exactly once.
    pub game: Option<GameIndex>,
    /// Most recent game seen among the games of the move.
    latest: Option<GameIndex>,
}

impl PreparedMove {
    fn new(m: PackedUciMove) -> PreparedMove {
        PreparedMove {
            m,
            stats: SubStats::default(),
            game: None,
            latest: None,
        }
    }

    fn see_game(&mut self, id: GameIndex) {
        self.latest = self.latest.max(Some(id));
    }

    /// Finishes the move, once all its cells are aggregated: whether it has
    /// any games, and which game is the only one.
    fn finish(mut self) -> Option<PreparedMove> {
        if self.stats.games() == 0 {
            return None;
        }
        self.game = self.latest.filter(|_| self.stats.games() == 1);
        Some(self)
    }
}

/// A game referenced by an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameRef {
    pub group: RatingGroup,
    pub speed: Option<Speed>,
    pub m: PackedUciMove,
    /// Chronological within the entry.
    pub id: GameIndex,
}

#[derive(Debug, Clone)]
pub struct HistorySegment {
    pub month: Month,
    pub stats: SubStats,
}

pub(crate) struct Ranked<K, T> {
    key: K,
    item: T,
}

impl<K: Ord, T> PartialEq for Ranked<K, T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl<K: Ord, T> Eq for Ranked<K, T> {}

impl<K: Ord, T> PartialOrd for Ranked<K, T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: Ord, T> Ord for Ranked<K, T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

/// Keeps the `k` items with the largest keys (ties broken arbitrarily).
pub(crate) struct TopK<K: Ord, T> {
    k: usize,
    heap: BinaryHeap<Reverse<Ranked<K, T>>>,
}

impl<K: Ord, T> TopK<K, T> {
    pub(crate) fn new(k: usize) -> TopK<K, T> {
        TopK {
            k,
            heap: BinaryHeap::with_capacity(k + 1),
        }
    }

    /// Whether an item with the key would be kept.
    pub(crate) fn accepts(&self, key: &K) -> bool {
        self.k > 0
            && (self.heap.len() < self.k
                || self
                    .heap
                    .peek()
                    .is_some_and(|Reverse(worst)| worst.key < *key))
    }

    pub(crate) fn push(&mut self, key: K, item: T) {
        if !self.accepts(&key) {
            return;
        }
        if self.heap.len() >= self.k {
            self.heap.pop();
        }
        self.heap.push(Reverse(Ranked { key, item }));
    }

    /// The items, best first.
    pub(crate) fn into_sorted(self) -> Vec<T> {
        let mut items: Vec<Ranked<K, T>> = self.heap.into_iter().map(|Reverse(r)| r).collect();
        items.sort_by(|a, b| b.key.cmp(&a.key));
        items.into_iter().map(|r| r.item).collect()
    }
}

/// Highest rating ever reached at the speed (as of 2024-07-27), to rank top
/// games across speeds.
fn highscore(speed: Option<Speed>) -> i32 {
    match speed {
        Some(Speed::UltraBullet) => 2644,
        Some(Speed::Bullet) => 3352,
        Some(Speed::Blitz) => 2974,
        Some(Speed::Rapid) => 2949,
        Some(Speed::Classical) => 2533,
        Some(Speed::Correspondence) | None => 4000,
    }
}

fn lower_bound(group: RatingGroup) -> i32 {
    match group {
        RatingGroup::Low => 0,
        RatingGroup::Group1000 => 1000,
        RatingGroup::Group1200 => 1200,
        RatingGroup::Group1400 => 1400,
        RatingGroup::Group1600 => 1600,
        RatingGroup::Group1800 => 1800,
        RatingGroup::Group2000 => 2000,
        RatingGroup::Group2200 => 2200,
        RatingGroup::Group2500 => 2500,
        RatingGroup::Group2800 => 2800,
        RatingGroup::Group3200 => 3200,
    }
}

/// Key of a top game: closest to the highscore of its speed, then most
/// recent.
pub(crate) fn top_key(
    speed: Option<Speed>,
    group: RatingGroup,
    id: GameIndex,
) -> (Reverse<i32>, GameIndex) {
    (Reverse(highscore(speed) - lower_bound(group)), id)
}

/// Which games of the matching cells to keep while aggregating.
pub(crate) struct GameSelection {
    /// Most recent games, enough to fill the recent games after removing
    /// the top games.
    pub(crate) recent: TopK<GameIndex, GameRef>,
    /// Best games among the top rating groups.
    pub(crate) top: TopK<(Reverse<i32>, GameIndex), GameRef>,
    pub(crate) top_group: Option<RatingGroup>,
    /// Masters: most recent games of the highest groups.
    pub(crate) candidates: TopK<(RatingGroup, GameIndex), GameRef>,
}

impl GameSelection {
    /// Whether any game of a cell whose newest game is the given one could
    /// be selected.
    pub(crate) fn accepts_cell(
        &self,
        speed: Option<Speed>,
        group: RatingGroup,
        newest: GameIndex,
    ) -> bool {
        self.recent.accepts(&newest)
            || (self.top_group.is_some_and(|top_group| group >= top_group)
                && self.top.accepts(&top_key(speed, group, newest)))
            || self.candidates.accepts(&(group, newest))
    }

    pub(crate) fn push(&mut self, g: GameRef) {
        self.recent.push(g.id, g);
        if self.top_group.is_some_and(|top_group| g.group >= top_group) {
            self.top.push(top_key(g.speed, g.group, g.id), g);
        }
        self.candidates.push((g.group, g.id), g);
    }
}

#[derive(Default)]
struct Aggregate {
    total: SubStats,
    moves: Vec<PreparedMove>,
    months: BTreeMap<Month, SubStats>,
}

impl Aggregate {
    fn add(&mut self, pm: &mut PreparedMove, month: Month, stats: &SubStats, collect_months: bool) {
        pm.stats.merge(stats);
        self.total.merge(stats);
        if collect_months {
            self.months.entry(month).or_default().merge(stats);
        }
    }
}

/// Aggregates the cells of a packed entry that match the filter, reading
/// as little as the format allows: groups that the filter does not select
/// are skipped, the subtotal of a group stands in for its cells when there
/// is no month range, and the cells of a group (newest first) are left
/// once they are older than `since`, or once their games can no longer be
/// selected and the subtotal already counted them.
fn aggregate(
    bytes: &[u8],
    filter: &LichessFilter,
    mut games: Option<&mut GameSelection>,
    collect_months: bool,
) -> Aggregate {
    let mut agg = Aggregate::default();
    let no_range = filter.since.is_none() && filter.until.is_none();
    let mut current: Option<PreparedMove> = None;
    for group in EntryView::<GroupHeader>::unpack_entire(bytes).groups() {
        let m = group.uci_move();
        if current.as_ref().is_none_or(|pm| pm.m != m) {
            agg.moves
                .extend(current.take().and_then(PreparedMove::finish));
            current = Some(PreparedMove::new(m));
        }
        let pm = current.as_mut().expect("current move");
        let header = group.header;
        if !filter.contains_speed(header.speed)
            || !filter.contains_rating_group(header.rating_group)
        {
            continue;
        }

        let subtotal = group.subtotal.filter(|_| no_range && !collect_months);
        if let Some(subtotal) = subtotal {
            agg.add(pm, group.newest_month, &subtotal, false);
        }

        let mut games_done = games.is_none();
        let mut cells = group.cells();
        while let Some(cell) = cells.next_cell() {
            let month = cell.key.month();
            if filter.since.is_some_and(|since| month < since) {
                break;
            }
            if subtotal.is_some() && games_done {
                break;
            }
            if !filter.contains_month(month) {
                continue;
            }
            if subtotal.is_none() {
                agg.add(pm, month, &cell.stats, collect_months);
            }
            if games_done {
                continue;
            }
            let games = games.as_deref_mut().expect("games wanted");
            let Some(id) = cells.next_game() else {
                continue;
            };
            pm.see_game(id);
            if !games.accepts_cell(header.speed, header.rating_group, id) {
                games_done = true;
                continue;
            }
            let game_ref = GameRef {
                group: header.rating_group,
                speed: header.speed,
                m,
                id,
            };
            games.push(game_ref);
            while let Some(id) = cells.next_game() {
                games.push(GameRef { id, ..game_ref });
            }
        }
    }
    agg.moves.extend(current.and_then(PreparedMove::finish));
    agg
}

pub(crate) fn sort_moves_and_truncate(moves: &mut Vec<PreparedMove>, num: usize) {
    let num = num.min(moves.len());
    moves.partial_sort(num, |a, b| b.stats.games().cmp(&a.stats.games()));
    moves.truncate(num);
}

#[derive(Debug)]
pub struct LichessPrepared {
    pub total: SubStats,
    pub moves: Vec<PreparedMove>,
    pub top_games: Vec<GameRef>,
    pub recent_games: Vec<GameRef>,
    pub history: Option<Vec<HistorySegment>>,
}

/// Prepares a lichess response from a packed entry.
pub fn prepare_lichess(
    bytes: &[u8],
    filter: &LichessFilter,
    limits: &Limits,
    history: bool,
) -> LichessPrepared {
    prepare_lichess_with(
        |games, collect_months| aggregate(bytes, filter, games, collect_months),
        filter,
        limits,
        history,
    )
}

/// Prepares a lichess response with the given aggregation function.
fn prepare_lichess_with(
    aggregate: impl FnOnce(Option<&mut GameSelection>, bool) -> Aggregate,
    filter: &LichessFilter,
    limits: &Limits,
    history: bool,
) -> LichessPrepared {
    let mut games = GameSelection {
        recent: TopK::new(MAX_LICHESS_GAMES + MAX_TOP_GAMES),
        top: TopK::new(MAX_TOP_GAMES),
        top_group: filter.top_group(),
        candidates: TopK::new(0),
    };
    let agg = aggregate(limits.games_wanted().then_some(&mut games), history);
    let mut moves = agg.moves;
    sort_moves_and_truncate(&mut moves, limits.moves);

    let mut top_games = games.top.into_sorted();
    let valid_recent_games = MAX_LICHESS_GAMES - top_games.len();
    let mut recent_games: Vec<GameRef> = games
        .recent
        .into_sorted()
        .into_iter()
        .filter(|g| !top_games.iter().any(|t| t.id == g.id))
        .collect();
    top_games.truncate(limits.top_games);
    recent_games.truncate(min(valid_recent_games, limits.recent_games));

    LichessPrepared {
        total: agg.total,
        moves,
        top_games,
        recent_games,
        history: history.then(|| build_history(agg.months, filter)),
    }
}

/// Monthly statistics from the first month (or `since`) to the last month,
/// with gaps filled. Without `until` the last month is omitted, because it
/// may not be completely indexed.
pub(crate) fn build_history(
    months: BTreeMap<Month, SubStats>,
    filter: &LichessFilter,
) -> Vec<HistorySegment> {
    let mut segments = Vec::with_capacity(months.len());
    let mut next = filter.since;
    for (month, stats) in months {
        if let Some(mut gap) = next {
            while gap < month {
                segments.push(HistorySegment {
                    month: gap,
                    stats: SubStats::default(),
                });
                gap = Month::from_months_since_jan_1900(gap.months_since_jan_1900() + 1);
            }
        }
        segments.push(HistorySegment { month, stats });
        next = Some(Month::from_months_since_jan_1900(
            month.months_since_jan_1900() + 1,
        ));
    }
    if filter.until.is_none() {
        segments.pop();
    }
    segments
}

#[derive(Debug)]
pub struct MastersPrepared {
    pub total: SubStats,
    pub moves: Vec<PreparedMove>,
    /// Candidates for the top games, best groups and most recent first.
    pub candidates: Vec<GameRef>,
}

/// Aggregates a masters entry. Top games are the highest rated ones, but
/// entries only keep the most recent games of each cell, so the candidates
/// are the most recent games of the highest rating groups, up to
/// `max_candidates`, to be ranked by their actual ratings.
pub fn prepare_masters(
    bytes: &[u8],
    filter: &LichessFilter,
    limits: &Limits,
    max_candidates: usize,
) -> MastersPrepared {
    prepare_masters_with(
        |games, collect_months| aggregate(bytes, filter, games, collect_months),
        limits,
        max_candidates,
    )
}

/// Prepares a masters response with the given aggregation function.
fn prepare_masters_with(
    aggregate: impl FnOnce(Option<&mut GameSelection>, bool) -> Aggregate,
    limits: &Limits,
    max_candidates: usize,
) -> MastersPrepared {
    let mut games = GameSelection {
        recent: TopK::new(0),
        top: TopK::new(0),
        top_group: None,
        candidates: TopK::new(max_candidates),
    };
    let agg = aggregate((limits.top_games > 0).then_some(&mut games), false);
    let mut moves = agg.moves;
    sort_moves_and_truncate(&mut moves, limits.moves);
    MastersPrepared {
        total: agg.total,
        moves,
        candidates: games.candidates.into_sorted(),
    }
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;
    use shakmaty::{Color, KnownOutcome, uci::UciMove};

    use super::*;
    use crate::packed::{CellKey, GroupHeader, LichessEntry, Rating, RatingGroups, Speeds};

    fn id(n: u64) -> GameIndex {
        GameIndex::new(n).unwrap()
    }

    fn entry() -> Vec<u8> {
        let mut entry = LichessEntry::new();
        let mut seq = 0;
        let mut add = |uci: &str, speed: Speed, rating: i32, outcome: KnownOutcome, month: u16| {
            let rating = Rating::try_from(rating).unwrap();
            seq += 1;
            entry.add_game(
                CellKey::new(
                    PackedUciMove::pack(uci.parse::<UciMove>().unwrap()),
                    GroupHeader {
                        speed: Some(speed),
                        rating_group: RatingGroup::select(rating),
                    },
                    Month::from_year_and_month(2020, month).unwrap(),
                ),
                rating,
                outcome,
                id(seq),
            );
        };
        let white = KnownOutcome::Decisive {
            winner: Color::White,
        };
        add("e2e4", Speed::Blitz, 1500, white, 1);
        add("e2e4", Speed::Blitz, 1600, KnownOutcome::Draw, 2);
        add("e2e4", Speed::Bullet, 2600, white, 3);
        add("d2d4", Speed::Rapid, 2100, white, 3);
        let mut buf = Vec::new();
        entry.pack_entire(&mut buf);
        buf
    }

    fn all() -> LichessFilter {
        LichessFilter {
            speeds: None,
            ratings: None,
            since: None,
            until: None,
        }
    }

    fn limits() -> Limits {
        Limits {
            moves: 12,
            top_games: usize::MAX,
            recent_games: usize::MAX,
        }
    }

    #[test]
    fn test_top_k() {
        let mut top = TopK::new(2);
        for (key, item) in [(3, "c"), (1, "a"), (5, "e"), (4, "d"), (2, "b")] {
            top.push(key, item);
        }
        assert!(top.accepts(&5) && !top.accepts(&4));
        assert_eq!(top.into_sorted(), vec!["e", "d"]);
        let mut none: TopK<i32, i32> = TopK::new(0);
        none.push(1, 1);
        assert!(none.into_sorted().is_empty());
    }

    #[test]
    fn test_prepare_lichess() {
        let buf = entry();
        let prepared = prepare_lichess(&buf, &all(), &limits(), true);
        assert_eq!(prepared.total.games(), 4);
        assert_eq!(prepared.moves.len(), 2);
        assert_eq!(prepared.moves[0].stats.games(), 3);
        assert_eq!(prepared.moves[0].game, None);
        assert_eq!(prepared.moves[1].game, Some(id(4)));
        // Without a rating filter, the top groups down to 2000 are selected:
        // games 3 (2600 bullet) and 4 (2100 rapid) are top games, with the
        // bullet game closer to the highscore of its speed.
        let top: Vec<GameIndex> = prepared.top_games.iter().map(|g| g.id).collect();
        assert_eq!(top, vec![id(3), id(4)]);
        let recent: Vec<GameIndex> = prepared.recent_games.iter().map(|g| g.id).collect();
        assert_eq!(recent, vec![id(2), id(1)]);
        let history = prepared.history.unwrap();
        // Without `until`, the last month is omitted.
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].stats.games(), 1);
        assert_eq!(history[1].stats.games(), 1);

        let filter = LichessFilter {
            speeds: Some(Speeds::BLITZ),
            ..all()
        };
        let prepared = prepare_lichess(&buf, &filter, &limits(), false);
        assert_eq!(prepared.total.games(), 2);
        assert_eq!(prepared.moves.len(), 1);
        assert_eq!(prepared.recent_games.len(), 2);

        let filter = LichessFilter {
            ratings: Some(
                RatingGroups::GROUP_2000
                    | RatingGroups::GROUP_2200
                    | RatingGroups::GROUP_2500
                    | RatingGroups::GROUP_2800
                    | RatingGroups::GROUP_3200,
            ),
            since: Some(Month::from_year_and_month(2020, 3).unwrap()),
            until: Some(Month::from_year_and_month(2020, 3).unwrap()),
            ..all()
        };
        let prepared = prepare_lichess(&buf, &filter, &limits(), true);
        assert_eq!(prepared.total.games(), 2);
        assert_eq!(prepared.top_games.len(), 2);
        assert!(prepared.recent_games.is_empty());
        assert_eq!(prepared.history.unwrap().len(), 1);

        let no_games = Limits {
            top_games: 0,
            recent_games: 0,
            moves: 1,
        };
        let prepared = prepare_lichess(&buf, &all(), &no_games, false);
        assert_eq!(prepared.moves.len(), 1);
        assert!(prepared.top_games.is_empty() && prepared.recent_games.is_empty());
    }

    #[test]
    fn test_prepare_masters() {
        let buf = entry();
        let prepared = prepare_masters(&buf, &all(), &limits(), 2);
        assert_eq!(prepared.total.games(), 4);
        let candidates: Vec<GameIndex> = prepared.candidates.iter().map(|g| g.id).collect();
        assert_eq!(candidates, vec![id(3), id(4)]);
    }

    fn game_ids(games: &[GameRef]) -> Vec<GameIndex> {
        games.iter().map(|g| g.id).collect()
    }

    /// Reference aggregation: every cell of the entry, nothing skipped.
    fn aggregate_naive(
        bytes: &[u8],
        filter: &LichessFilter,
        mut games: Option<&mut GameSelection>,
        collect_months: bool,
    ) -> Aggregate {
        let mut agg = Aggregate::default();
        let mut by_move: BTreeMap<[u8; 2], PreparedMove> = BTreeMap::new();
        for cell in EntryView::<GroupHeader>::unpack_entire(bytes).cells() {
            let header: GroupHeader = cell.key.group_header();
            let month = cell.key.month();
            if !filter.contains_speed(header.speed)
                || !filter.contains_rating_group(header.rating_group)
                || !filter.contains_month(month)
            {
                continue;
            }
            let m = cell.key.uci_move();
            let pm = by_move
                .entry(m.to_bytes())
                .or_insert_with(|| PreparedMove::new(m));
            agg.add(pm, month, &cell.stats, collect_months);
            if let Some(games) = games.as_deref_mut() {
                for id in cell.games {
                    pm.see_game(id);
                    games.push(GameRef {
                        group: header.rating_group,
                        speed: header.speed,
                        m,
                        id,
                    });
                }
            }
        }
        agg.moves = by_move
            .into_values()
            .filter_map(PreparedMove::finish)
            .collect();
        agg
    }

    fn moves(moves: &[PreparedMove]) -> Vec<(PackedUciMove, SubStats, Option<GameIndex>)> {
        moves.iter().map(|pm| (pm.m, pm.stats, pm.game)).collect()
    }

    /// An entry with few moves and groups but many months, so that groups
    /// have subtotals and early stops matter.
    fn dense_entry(u: &mut arbitrary::Unstructured) -> arbitrary::Result<LichessEntry> {
        let mut entry = LichessEntry::new();
        let moves = ["e2e4", "d2d4", "c2c4"];
        // Chronological game indexes, as in real entries.
        let mut month = 1400;
        for seq in 0..u.int_in_range(0..=400)? {
            if u.ratio(1, 6)? {
                month += 1;
            }
            let uci: UciMove = moves[u.int_in_range(0..=2)?].parse().unwrap();
            let speed = if u.arbitrary()? {
                Speed::Blitz
            } else {
                Speed::Rapid
            };
            let rating = Rating::try_from(u.int_in_range(1500..=2600)? / 500 * 500).unwrap();
            let outcome: KnownOutcome = u.arbitrary()?;
            let month = Month::from_months_since_jan_1900(month);
            entry.add_game(
                CellKey::new(
                    PackedUciMove::pack(uci),
                    GroupHeader {
                        speed: Some(speed),
                        rating_group: RatingGroup::select(rating),
                    },
                    month,
                ),
                rating,
                outcome,
                id(seq + 1),
            );
        }
        Ok(entry)
    }

    /// Skipping groups, using subtotals and stopping early gives the same
    /// results as aggregating every cell.
    #[test]
    fn test_matches_naive() {
        arbtest(|u| {
            let mut entry: LichessEntry = if u.arbitrary()? {
                u.arbitrary()?
            } else {
                dense_entry(u)?
            };
            let mut buf = Vec::new();
            entry.pack_entire(&mut buf);
            let subtotals = EntryView::<GroupHeader>::unpack_entire(&buf)
                .groups()
                .filter(|g| g.subtotal.is_some())
                .count();
            SUBTOTALS_SEEN.fetch_add(subtotals, std::sync::atomic::Ordering::Relaxed);
            let month = |u: &mut arbitrary::Unstructured| {
                Ok(Month::from_months_since_jan_1900(
                    u.int_in_range(1400..=1600)?,
                ))
            };
            let filter = LichessFilter {
                speeds: u.arbitrary::<Option<u8>>()?.map(Speeds::from_bits_truncate),
                ratings: u
                    .arbitrary::<Option<u16>>()?
                    .map(RatingGroups::from_bits_truncate),
                since: if u.arbitrary()? {
                    Some(month(u)?)
                } else {
                    None
                },
                until: if u.arbitrary()? {
                    Some(month(u)?)
                } else {
                    None
                },
            };
            let limits = Limits {
                moves: 12,
                top_games: u.int_in_range(0..=4)?,
                recent_games: u.int_in_range(0..=8)?,
            };
            let history: bool = u.arbitrary()?;
            let a = prepare_lichess(&buf, &filter, &limits, history);
            let b = prepare_lichess_with(
                |games, collect_months| aggregate_naive(&buf, &filter, games, collect_months),
                &filter,
                &limits,
                history,
            );
            assert_eq!(a.total, b.total);
            assert_eq!(moves(&a.moves), moves(&b.moves));
            // Ties do not occur: game indexes are unique within an entry.
            assert_eq!(game_ids(&a.top_games), game_ids(&b.top_games));
            assert_eq!(game_ids(&a.recent_games), game_ids(&b.recent_games));
            let segments = |p: &LichessPrepared| {
                p.history
                    .as_ref()
                    .map(|h| h.iter().map(|s| (s.month, s.stats)).collect::<Vec<_>>())
            };
            assert_eq!(segments(&a), segments(&b));

            let candidates = u.int_in_range(0..=5)?;
            let a = prepare_masters(&buf, &filter, &limits, candidates);
            let b = prepare_masters_with(
                |games, collect_months| aggregate_naive(&buf, &filter, games, collect_months),
                &limits,
                candidates,
            );
            assert_eq!(a.total, b.total);
            assert_eq!(moves(&a.moves), moves(&b.moves));
            assert_eq!(game_ids(&a.candidates), game_ids(&b.candidates));
            Ok(())
        })
        .budget_ms(2000);
        assert!(
            SUBTOTALS_SEEN.load(std::sync::atomic::Ordering::Relaxed) > 0,
            "subtotals exercised"
        );
    }

    static SUBTOTALS_SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
}
