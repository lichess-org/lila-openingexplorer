use std::{io::Write, iter, sync::Arc};

use bytes::Bytes;
use parquet::{
    basic::{Compression, LogicalType, Repetition, Type as PhysicalType},
    data_type::{ByteArray, ByteArrayType, FixedLenByteArray, FixedLenByteArrayType, Int32Type},
    file::{
        properties::{EnabledStatistics, WriterProperties, WriterPropertiesBuilder},
        reader::RowGroupReader,
        writer::{SerializedFileWriter, SerializedRowGroupWriter},
    },
    schema::types::{ColumnPath, Type, TypePtr},
};
use shakmaty::{ByColor, KnownOutcome};

use crate::{
    packed::{
        BorrowedEventInfo, BorrowedGamePlayer, BorrowedLichessGame, GameId, GamePlayer, LaxDate,
        LichessGame, Rating, RatingMode, Speed,
    },
    parquet::{OptionalColumn, RequiredColumn},
};

#[derive(Default)]
pub struct PlayerColumns {
    title: OptionalColumn<ByteArrayType>,
    name: RequiredColumn<ByteArrayType>,
    rating: OptionalColumn<Int32Type>,
}

impl PlayerColumns {
    pub fn clear(&mut self) {
        self.title.clear();
        self.name.clear();
        self.rating.clear();
    }

    pub fn reserve(&mut self, additional: usize) {
        self.title.reserve(additional);
        self.name.reserve(additional);
        self.rating.reserve(additional);
    }

    pub fn append(&mut self, other: &mut PlayerColumns) {
        self.title.append(&mut other.title);
        self.name.append(&mut other.name);
        self.rating.append(&mut other.rating);
    }

    pub fn push(&mut self, player: GamePlayer) {
        self.title.push(
            player
                .title
                .map(|title| ByteArray::from(title.into_bytes())),
        );
        self.name.push(ByteArray::from(player.name.into_bytes()));
        self.rating.push(player.rating.map(i32::from));
    }

    pub fn write_columns<W>(
        &self,
        row_group: &mut SerializedRowGroupWriter<'_, W>,
    ) -> parquet::errors::Result<()>
    where
        W: Write + Send,
    {
        self.title.write_column(row_group)?;
        self.name.write_column(row_group)?;
        self.rating.write_column(row_group)?;
        Ok(())
    }

    pub fn scatter(&self, partitions: &[Option<usize>], targets: &mut [&mut PlayerColumns]) {
        self.title
            .scatter(partitions, &mut project(targets, |t| &mut t.title));
        self.name
            .scatter(partitions, &mut project(targets, |t| &mut t.name));
        self.rating
            .scatter(partitions, &mut project(targets, |t| &mut t.rating));
    }

    pub fn read_columns(
        &mut self,
        reader: &dyn RowGroupReader,
        base_column_index: usize,
    ) -> parquet::errors::Result<()> {
        self.title.read_column(reader, base_column_index)?;
        self.name.read_column(reader, base_column_index + 1)?;
        self.rating.read_column(reader, base_column_index + 2)?;
        Ok(())
    }

    pub fn rows(&self) -> impl Iterator<Item = BorrowedGamePlayer<'_>> {
        let mut title = self.title.iter();
        let mut name = self.name.iter();
        let mut rating = self.rating.iter();
        iter::from_fn(move || {
            Some(BorrowedGamePlayer {
                name: str::from_utf8(name.next()?.data()).expect("valid utf-8"),
                title: title
                    .next()
                    .expect("title")
                    .map(|title| str::from_utf8(title.data()).expect("valid utf-8")),
                rating: rating
                    .next()
                    .expect("rating")
                    .copied()
                    .map(|rating| Rating::try_from(rating).expect("valid rating")),
            })
        })
    }
}

#[derive(Default)]
pub struct GameColumns {
    game_id: RequiredColumn<FixedLenByteArrayType>, // 0
    players: ByColor<PlayerColumns>,                // 1,2,3 / 4,5,6
    date_year: RequiredColumn<Int32Type>,           // 7
    date_month: OptionalColumn<Int32Type>,          // 8
    date_day: OptionalColumn<Int32Type>,            // 9
    outcome: RequiredColumn<ByteArrayType>,         // 10
    speed: OptionalColumn<ByteArrayType>,           // 11
    rating_mode: RequiredColumn<ByteArrayType>,     // 12
    setup: OptionalColumn<ByteArrayType>,           // 13
    moves: RequiredColumn<ByteArrayType>,           // 14
    // Event tags, only for external collections like the masters database.
    // Missing entirely from older files.
    event: OptionalColumn<ByteArrayType>, // 15
    site: OptionalColumn<ByteArrayType>,  // 16
    round: OptionalColumn<ByteArrayType>, // 17
}

/// Number of columns of files without event tags.
const COLUMNS_WITHOUT_EVENT: usize = 15;

impl GameColumns {
    pub fn is_empty(&self) -> bool {
        self.game_id.is_empty()
    }

    pub fn len(&self) -> usize {
        self.game_id.len()
    }

    pub fn clear(&mut self) {
        self.game_id.clear();
        self.players.white.clear();
        self.players.black.clear();
        self.date_year.clear();
        self.date_month.clear();
        self.date_day.clear();
        self.outcome.clear();
        self.speed.clear();
        self.rating_mode.clear();
        self.setup.clear();
        self.moves.clear();
        self.event.clear();
        self.site.clear();
        self.round.clear();
    }

    pub fn reserve(&mut self, additional: usize) {
        self.game_id.reserve(additional);
        self.players.white.reserve(additional);
        self.players.black.reserve(additional);
        self.date_year.reserve(additional);
        self.date_month.reserve(additional);
        self.date_day.reserve(additional);
        self.outcome.reserve(additional);
        self.speed.reserve(additional);
        self.rating_mode.reserve(additional);
        self.setup.reserve(additional);
        self.moves.reserve(additional);
        self.event.reserve(additional);
        self.site.reserve(additional);
        self.round.reserve(additional);
    }

    pub fn append(&mut self, other: &mut GameColumns) {
        self.game_id.append(&mut other.game_id);
        self.players.white.append(&mut other.players.white);
        self.players.black.append(&mut other.players.black);
        self.date_year.append(&mut other.date_year);
        self.date_month.append(&mut other.date_month);
        self.date_day.append(&mut other.date_day);
        self.outcome.append(&mut other.outcome);
        self.speed.append(&mut other.speed);
        self.rating_mode.append(&mut other.rating_mode);
        self.setup.append(&mut other.setup);
        self.moves.append(&mut other.moves);
        self.event.append(&mut other.event);
        self.site.append(&mut other.site);
        self.round.append(&mut other.round);
    }

    pub fn rows(&self) -> impl Iterator<Item = (GameId, BorrowedLichessGame<'_>)> {
        let mut game_id = self.game_id.iter();
        let mut white = self.players.white.rows();
        let mut black = self.players.black.rows();
        let mut date_year = self.date_year.iter();
        let mut date_month = self.date_month.iter();
        let mut date_day = self.date_day.iter();
        let mut outcome = self.outcome.iter();
        let mut speed = self.speed.iter();
        let mut rating_mode = self.rating_mode.iter();
        let mut setup = self.setup.iter();
        let mut moves = self.moves.iter();
        let mut event = self.event.iter();
        let mut site = self.site.iter();
        let mut round = self.round.iter();
        iter::from_fn(move || {
            Some((
                GameId::unpack(&mut game_id.next()?.data()),
                BorrowedLichessGame {
                    players: ByColor {
                        white: white.next().expect("white"),
                        black: black.next().expect("black"),
                    },
                    date: LaxDate::from_ymd(
                        u32::try_from(*date_year.next().expect("date_year")).expect("valid year"),
                        date_month
                            .next()
                            .expect("date_month")
                            .map(|&month| u8::try_from(month).expect("valid month")),
                        date_day
                            .next()
                            .expect("date_day")
                            .map(|&day| u8::try_from(day).expect("valid day")),
                    )
                    .expect("valid date"),
                    outcome: KnownOutcome::from_ascii(outcome.next().expect("outcome").data())
                        .expect("known outcome"),
                    speed: speed
                        .next()
                        .expect("speed")
                        .map(|speed| Speed::from_ascii(speed.data()).expect("valid speed")),
                    rating_mode: RatingMode::from_ascii(
                        rating_mode.next().expect("rating_mode").data(),
                    )
                    .expect("valid rating mode"),
                    // Empty setups are treated like missing setups.
                    setup: setup
                        .next()
                        .expect("setup")
                        .map(ByteArray::data)
                        .filter(|bytes| !bytes.is_empty()),
                    event: match (
                        event.next().expect("event"),
                        site.next().expect("site"),
                        round.next().expect("round"),
                    ) {
                        (Some(event), Some(site), Some(round)) => Some(BorrowedEventInfo {
                            event: utf8(event),
                            site: utf8(site),
                            round: utf8(round),
                        }),
                        (None, None, None) => None,
                        _ => panic!("partial event info"),
                    },
                    moves: moves.next().expect("moves").data(),
                },
            ))
        })
    }

    pub fn push(&mut self, game_id: GameId, game: LichessGame) {
        self.game_id
            .push(FixedLenByteArray::from(game_id.to_bytes().to_vec()));

        self.players.white.push(game.players.white);
        self.players.black.push(game.players.black);

        self.date_year.push(game.date.year() as i32);
        self.date_month.push(game.date.month().map(i32::from));
        self.date_day.push(game.date.day().map(i32::from));

        self.outcome.push(ByteArray::from(Bytes::from_static(
            game.outcome.as_str().as_bytes(),
        )));
        self.speed.push(
            game.speed
                .map(|speed| ByteArray::from(Bytes::from_static(speed.as_str().as_bytes()))),
        );
        self.rating_mode.push(ByteArray::from(Bytes::from_static(
            game.rating_mode.as_str().as_bytes(),
        )));

        self.setup
            .push(game.setup.map(|setup| ByteArray::from(setup.as_bytes())));

        self.moves.push(ByteArray::from(game.moves.0));

        match game.event {
            Some(event) => {
                self.event
                    .push(Some(ByteArray::from(event.event.into_bytes())));
                self.site
                    .push(Some(ByteArray::from(event.site.into_bytes())));
                self.round
                    .push(Some(ByteArray::from(event.round.into_bytes())));
            }
            None => {
                self.event.push(None);
                self.site.push(None);
                self.round.push(None);
            }
        }
    }

    pub fn write_row_group<W>(
        &self,
        writer: &mut SerializedFileWriter<W>,
    ) -> parquet::errors::Result<()>
    where
        W: Write + Send,
    {
        let mut row_group = writer.next_row_group()?;
        self.game_id.write_column(&mut row_group)?;
        self.players.white.write_columns(&mut row_group)?;
        self.players.black.write_columns(&mut row_group)?;
        self.date_year.write_column(&mut row_group)?;
        self.date_month.write_column(&mut row_group)?;
        self.date_day.write_column(&mut row_group)?;
        self.outcome.write_column(&mut row_group)?;
        self.speed.write_column(&mut row_group)?;
        self.rating_mode.write_column(&mut row_group)?;
        self.setup.write_column(&mut row_group)?;
        self.moves.write_column(&mut row_group)?;
        self.event.write_column(&mut row_group)?;
        self.site.write_column(&mut row_group)?;
        self.round.write_column(&mut row_group)?;
        row_group.close()?;
        Ok(())
    }

    pub fn read_columns(&mut self, reader: &dyn RowGroupReader) -> parquet::errors::Result<()> {
        self.game_id.read_column(reader, 0)?;
        self.players.white.read_columns(reader, 1)?;
        self.players.black.read_columns(reader, 4)?;
        self.date_year.read_column(reader, 7)?;
        self.date_month.read_column(reader, 8)?;
        self.date_day.read_column(reader, 9)?;
        self.outcome.read_column(reader, 10)?;
        self.speed.read_column(reader, 11)?;
        self.rating_mode.read_column(reader, 12)?;
        self.setup.read_column(reader, 13)?;
        self.moves.read_column(reader, 14)?;
        if reader.num_columns() > COLUMNS_WITHOUT_EVENT {
            self.event.read_column(reader, 15)?;
            self.site.read_column(reader, 16)?;
            self.round.read_column(reader, 17)?;
        } else {
            for _ in 0..self.moves.len() {
                self.event.push(None);
                self.site.push(None);
                self.round.push(None);
            }
        }
        Ok(())
    }

    pub fn scatter(
        &self,
        mut partition: impl FnMut(GameId) -> Option<usize>,
        targets: &mut [&mut GameColumns],
    ) {
        let partitions: Vec<Option<usize>> = self
            .game_id
            .iter()
            .map(|game_id| partition(GameId::unpack(&mut game_id.data())))
            .collect();

        self.game_id
            .scatter(&partitions, &mut project(targets, |t| &mut t.game_id));
        self.players
            .white
            .scatter(&partitions, &mut project(targets, |t| &mut t.players.white));
        self.players
            .black
            .scatter(&partitions, &mut project(targets, |t| &mut t.players.black));
        self.date_year
            .scatter(&partitions, &mut project(targets, |t| &mut t.date_year));
        self.date_month
            .scatter(&partitions, &mut project(targets, |t| &mut t.date_month));
        self.date_day
            .scatter(&partitions, &mut project(targets, |t| &mut t.date_day));
        self.outcome
            .scatter(&partitions, &mut project(targets, |t| &mut t.outcome));
        self.speed
            .scatter(&partitions, &mut project(targets, |t| &mut t.speed));
        self.rating_mode
            .scatter(&partitions, &mut project(targets, |t| &mut t.rating_mode));
        self.setup
            .scatter(&partitions, &mut project(targets, |t| &mut t.setup));
        self.moves
            .scatter(&partitions, &mut project(targets, |t| &mut t.moves));
        self.event
            .scatter(&partitions, &mut project(targets, |t| &mut t.event));
        self.site
            .scatter(&partitions, &mut project(targets, |t| &mut t.site));
        self.round
            .scatter(&partitions, &mut project(targets, |t| &mut t.round));
    }

    pub fn schema() -> TypePtr {
        Arc::new(
            Type::group_type_builder("game")
                .with_fields(vec![
                    fixed_binary("game_id", GameId::SIZE as i32, Repetition::REQUIRED),
                    string("white_title", Repetition::OPTIONAL),
                    string("white_name", Repetition::REQUIRED),
                    int32("white_rating", Repetition::OPTIONAL),
                    string("black_title", Repetition::OPTIONAL),
                    string("black_name", Repetition::REQUIRED),
                    int32("black_rating", Repetition::OPTIONAL),
                    int32("date_year", Repetition::REQUIRED),
                    int32("date_month", Repetition::OPTIONAL),
                    int32("date_day", Repetition::OPTIONAL),
                    string("outcome", Repetition::REQUIRED),
                    string("speed", Repetition::OPTIONAL),
                    string("rating_mode", Repetition::REQUIRED),
                    binary("setup", Repetition::OPTIONAL),
                    binary("moves", Repetition::REQUIRED),
                    string("event", Repetition::OPTIONAL),
                    string("site", Repetition::OPTIONAL),
                    string("round", Repetition::OPTIONAL),
                ])
                .build()
                .expect("group type"),
        )
    }

    pub fn properties() -> WriterPropertiesBuilder {
        WriterProperties::builder()
            .set_data_page_size_limit(8 * 1024 * 1024)
            .set_data_page_row_count_limit(usize::MAX)
            .set_column_dictionary_enabled(ColumnPath::from("game_id"), false)
            .set_column_statistics_enabled(ColumnPath::from("game_id"), EnabledStatistics::None)
            .set_column_compression(ColumnPath::from("game_id"), Compression::UNCOMPRESSED) // Random ids
            .set_column_dictionary_enabled(ColumnPath::from("moves"), false)
            .set_column_statistics_enabled(ColumnPath::from("moves"), EnabledStatistics::None)
            .set_column_compression(ColumnPath::from("moves"), Compression::UNCOMPRESSED) // ~%7 for zstd -9 not worth it
            .set_column_dictionary_enabled(ColumnPath::from("setup"), false)
            .set_column_statistics_enabled(ColumnPath::from("setup"), EnabledStatistics::None)
    }
}

fn utf8(bytes: &ByteArray) -> &str {
    str::from_utf8(bytes.data()).expect("valid utf-8")
}

fn project<'a, 'b: 'a, S: 'b, T>(
    targets: &'a mut [&'b mut S],
    f: impl FnMut(&'a mut &'b mut S) -> &'a mut T,
) -> Vec<&'a mut T> {
    targets.iter_mut().map(f).collect()
}

fn primitive(
    name: &str,
    physical_type: PhysicalType,
    logical_type: Option<LogicalType>,
    repetition: Repetition,
) -> TypePtr {
    Arc::new(
        Type::primitive_type_builder(name, physical_type)
            .with_logical_type(logical_type)
            .with_repetition(repetition)
            .build()
            .expect("primitive type"),
    )
}

fn string(name: &str, repetition: Repetition) -> TypePtr {
    primitive(
        name,
        PhysicalType::BYTE_ARRAY,
        Some(LogicalType::String),
        repetition,
    )
}

fn int32(name: &str, repetition: Repetition) -> TypePtr {
    primitive(name, PhysicalType::INT32, None, repetition)
}

fn binary(name: &str, repetition: Repetition) -> TypePtr {
    primitive(name, PhysicalType::BYTE_ARRAY, None, repetition)
}

fn fixed_binary(name: &str, len: i32, repetition: Repetition) -> TypePtr {
    Arc::new(
        Type::primitive_type_builder(name, PhysicalType::FIXED_LEN_BYTE_ARRAY)
            .with_length(len)
            .with_repetition(repetition)
            .build()
            .expect("fixed len byte array type"),
    )
}

#[cfg(test)]
mod tests {
    use arbtest::arbtest;

    use super::*;

    #[test]
    fn test_rows_roundtrip() {
        arbtest(|u| {
            let game_id: GameId = u.arbitrary()?;
            let game: LichessGame = u.arbitrary()?;

            let mut columns = GameColumns::default();
            columns.push(game_id, game.clone());
            assert_eq!(
                columns.rows().next().expect("row"),
                (game_id, game.as_borrowed())
            );

            Ok(())
        });
    }
}
