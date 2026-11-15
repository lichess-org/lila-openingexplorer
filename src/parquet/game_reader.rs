use std::sync::Arc;

use arrow_array::{
    Array, BinaryArray, DictionaryArray, FixedSizeBinaryArray, Int32Array, RecordBatch,
    RecordBatchReader, StringArray, cast::AsArray, types::Int32Type,
};
use arrow_schema::{DataType, Schema};
use bytes::{Buf, Bytes};
use parquet::{
    arrow::{
        ProjectionMask,
        arrow_reader::{ArrowReaderMetadata, ArrowReaderOptions, ParquetRecordBatchReaderBuilder},
    },
    errors::{ParquetError, Result},
    file::{
        metadata::{ParquetMetaData, ParquetMetaDataReader},
        reader::{ChunkReader, Length},
    },
};
use shakmaty::{ByColor, KnownOutcome};

use crate::packed::{
    BorrowedEventInfo, BorrowedGamePlayer, BorrowedLichessGame, GameId, LaxDate, Rating,
    RatingMode, Speed,
};

const DICTIONARY_COLUMNS: [&str; 5] = [
    "white_title",
    "black_title",
    "outcome",
    "speed",
    "rating_mode",
];

pub struct GameReader<T> {
    input: T,
    metadata: ArrowReaderMetadata,
}

impl<T: ChunkReader + Clone + 'static> GameReader<T> {
    pub fn try_new(input: T) -> Result<GameReader<T>> {
        let metadata = ParquetMetaDataReader::new().parse_and_finish(&input)?;
        GameReader::new_with_metadata(input, Arc::new(metadata))
    }

    pub fn new_with_metadata(
        input: T,
        parquet_metadata: Arc<ParquetMetaData>,
    ) -> Result<GameReader<T>> {
        let schema = Schema::new(
            ArrowReaderMetadata::try_new(Arc::clone(&parquet_metadata), ArrowReaderOptions::new())?
                .schema()
                .fields()
                .iter()
                .map(|field| {
                    if DICTIONARY_COLUMNS.contains(&field.name().as_str()) {
                        Arc::new(field.as_ref().clone().with_data_type(DataType::Dictionary(
                            Box::new(DataType::Int32),
                            Box::new(DataType::Utf8),
                        )))
                    } else {
                        Arc::clone(field)
                    }
                })
                .collect::<Vec<_>>(),
        );
        let metadata = ArrowReaderMetadata::try_new(
            parquet_metadata,
            ArrowReaderOptions::new().with_schema(Arc::new(schema)),
        )?;
        Ok(GameReader { input, metadata })
    }

    pub fn metadata(&self) -> &ParquetMetaData {
        self.metadata.metadata()
    }

    pub fn input(&self) -> &T {
        &self.input
    }

    pub fn num_row_groups(&self) -> usize {
        self.metadata().num_row_groups()
    }

    fn read<R: ChunkReader + 'static>(
        &self,
        input: R,
        row_group: usize,
        projection: ProjectionMask,
    ) -> Result<RecordBatch> {
        let num_rows = self.metadata().row_group(row_group).num_rows() as usize;
        let mut reader =
            ParquetRecordBatchReaderBuilder::new_with_metadata(input, self.metadata.clone())
                .with_projection(projection)
                .with_row_groups(vec![row_group])
                .with_batch_size(num_rows.max(1))
                .build()?;
        let batch = match reader.next() {
            Some(batch) => batch?,
            None => RecordBatch::new_empty(reader.schema()),
        };
        debug_assert!(reader.next().is_none());
        Ok(batch)
    }

    pub fn read_games(&self, row_group: usize) -> Result<GameBatch> {
        self.read(self.input.clone(), row_group, ProjectionMask::all())
            .map(GameBatch)
    }

    /// Byte range of all column chunks of a row group.
    pub fn row_group_byte_range(&self, row_group: usize) -> (u64, u64) {
        let (mut start, mut end) = (u64::MAX, 0);
        for column in self.metadata().row_group(row_group).columns() {
            let (column_start, column_len) = column.byte_range();
            start = start.min(column_start);
            end = end.max(column_start + column_len);
        }
        (start.min(end), end)
    }

    /// Reads all bytes of a row group with a single read, so that
    /// `read_games_from` can decode it without touching the input again.
    pub fn fetch_row_group(&self, row_group: usize) -> Result<RowGroupBytes> {
        let (start, end) = self.row_group_byte_range(row_group);
        Ok(RowGroupBytes::new(
            start,
            self.input.len(),
            self.input.get_bytes(start, (end - start) as usize)?,
        ))
    }

    pub fn read_games_from(&self, row_group: usize, bytes: RowGroupBytes) -> Result<GameBatch> {
        self.read(bytes, row_group, ProjectionMask::all())
            .map(GameBatch)
    }

    pub fn read_game_ids(&self, row_group: usize) -> Result<Vec<GameId>> {
        let batch = self.read(
            self.input.clone(),
            row_group,
            ProjectionMask::columns(self.metadata.parquet_schema(), ["game_id"]),
        )?;

        Ok(batch
            .column(0)
            .as_fixed_size_binary()
            .value_data()
            .as_chunks::<{ GameId::SIZE }>()
            .0
            .iter()
            .map(|&game_id| GameId::try_from(game_id).expect("valid game id"))
            .collect())
    }
}

/// The bytes of one row group, serving reads at the file offsets they came
/// from.
#[derive(Clone)]
pub struct RowGroupBytes {
    start: u64,
    file_len: u64,
    bytes: Bytes,
}

impl RowGroupBytes {
    /// Wraps the bytes at offset `start` of a file of `file_len` bytes.
    pub fn new(start: u64, file_len: u64, bytes: Bytes) -> RowGroupBytes {
        RowGroupBytes {
            start,
            file_len,
            bytes,
        }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn slice(&self, start: u64, length: usize) -> Result<Bytes> {
        let end = self.start + self.bytes.len() as u64;
        match start.checked_sub(self.start) {
            Some(offset) if start + length as u64 <= end => {
                Ok(self.bytes.slice(offset as usize..offset as usize + length))
            }
            _ => Err(ParquetError::EOF(format!(
                "read of {length} bytes at {start} outside of row group bytes {}..{end}",
                self.start
            ))),
        }
    }
}

impl Length for RowGroupBytes {
    fn len(&self) -> u64 {
        self.file_len
    }
}

impl ChunkReader for RowGroupBytes {
    type T = bytes::buf::Reader<Bytes>;

    fn get_read(&self, start: u64) -> Result<Self::T> {
        let end = self.start + self.bytes.len() as u64;
        Ok(self
            .slice(start, end.saturating_sub(start) as usize)?
            .reader())
    }

    fn get_bytes(&self, start: u64, length: usize) -> Result<Bytes> {
        self.slice(start, length)
    }
}

pub struct GameBatch(RecordBatch);

impl GameBatch {
    #[inline]
    pub fn len(&self) -> usize {
        self.0.num_rows()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn rows(&self) -> impl Iterator<Item = (GameId, BorrowedLichessGame<'_>)> {
        let columns = BatchColumns::new(&self.0);
        (0..self.len()).map(move |i| columns.get(i))
    }
}

/// Event tag columns, missing from files without them.
struct EventColumns<'a> {
    event: &'a StringArray,
    site: &'a StringArray,
    round: &'a StringArray,
}

impl<'a> EventColumns<'a> {
    fn new(batch: &'a RecordBatch) -> Option<EventColumns<'a>> {
        let string = |name| Some(batch.column_by_name(name)?.as_string::<i32>());
        Some(EventColumns {
            event: string("event")?,
            site: string("site")?,
            round: string("round")?,
        })
    }

    /// Event tags of the game at index `i`, if it has any.
    #[inline]
    fn get(&self, i: usize) -> Option<BorrowedEventInfo<'a>> {
        self.event.is_valid(i).then(|| BorrowedEventInfo {
            event: self.event.value(i),
            site: self.site.value(i),
            round: self.round.value(i),
        })
    }
}

/// A dictionary encoded column, with the dictionary decoded once per batch.
struct DictionaryColumn<'a, T> {
    keys: &'a Int32Array,
    values: Vec<Option<T>>,
}

impl<'a, T: Copy> DictionaryColumn<'a, T> {
    fn new(batch: &'a RecordBatch, name: &str, decode: impl Fn(&'a str) -> Option<T>) -> Self {
        let array: &DictionaryArray<Int32Type> = column(batch, name).as_dictionary();
        DictionaryColumn {
            keys: array.keys(),
            values: array
                .values()
                .as_string::<i32>()
                .iter()
                .map(|value| value.and_then(&decode))
                .collect(),
        }
    }

    #[inline]
    fn get(&self, i: usize) -> Option<T> {
        self.keys
            .is_valid(i)
            .then(|| self.values[self.keys.value(i) as usize].expect("valid dictionary value"))
    }
}

struct PlayerColumns<'a> {
    title: DictionaryColumn<'a, &'a str>,
    name: &'a StringArray,
    rating: &'a Int32Array,
}

impl<'a> PlayerColumns<'a> {
    fn new(batch: &'a RecordBatch, color: &str) -> PlayerColumns<'a> {
        PlayerColumns {
            title: DictionaryColumn::new(batch, &format!("{color}_title"), Some),
            name: column(batch, &format!("{color}_name")).as_string(),
            rating: column(batch, &format!("{color}_rating")).as_primitive(),
        }
    }

    #[inline]
    fn get(&self, i: usize) -> BorrowedGamePlayer<'a> {
        BorrowedGamePlayer {
            title: self.title.get(i),
            name: self.name.value(i),
            rating: optional_int(self.rating, i)
                .map(|rating| Rating::try_from(rating).expect("valid rating")),
        }
    }
}

struct BatchColumns<'a> {
    game_id: &'a FixedSizeBinaryArray,
    players: ByColor<PlayerColumns<'a>>,
    date_year: &'a Int32Array,
    date_month: &'a Int32Array,
    date_day: &'a Int32Array,
    outcome: DictionaryColumn<'a, KnownOutcome>,
    speed: DictionaryColumn<'a, Speed>,
    rating_mode: DictionaryColumn<'a, RatingMode>,
    setup: &'a BinaryArray,
    event: Option<EventColumns<'a>>,
    moves: &'a BinaryArray,
}

impl<'a> BatchColumns<'a> {
    fn new(batch: &'a RecordBatch) -> BatchColumns<'a> {
        BatchColumns {
            game_id: column(batch, "game_id").as_fixed_size_binary(),
            players: ByColor {
                white: PlayerColumns::new(batch, "white"),
                black: PlayerColumns::new(batch, "black"),
            },
            date_year: column(batch, "date_year").as_primitive(),
            date_month: column(batch, "date_month").as_primitive(),
            date_day: column(batch, "date_day").as_primitive(),
            outcome: DictionaryColumn::new(batch, "outcome", |s| {
                KnownOutcome::from_ascii(s.as_bytes()).ok()
            }),
            speed: DictionaryColumn::new(batch, "speed", |s| Speed::from_ascii(s.as_bytes()).ok()),
            rating_mode: DictionaryColumn::new(batch, "rating_mode", |s| {
                RatingMode::from_ascii(s.as_bytes()).ok()
            }),
            setup: column(batch, "setup").as_binary(),
            event: EventColumns::new(batch),
            moves: column(batch, "moves").as_binary(),
        }
    }

    #[inline]
    fn get(&self, i: usize) -> (GameId, BorrowedLichessGame<'a>) {
        let game = BorrowedLichessGame {
            players: ByColor {
                white: self.players.white.get(i),
                black: self.players.black.get(i),
            },
            date: LaxDate::from_ymd(
                u32::try_from(self.date_year.value(i)).expect("valid year"),
                optional_int(self.date_month, i)
                    .map(|month| u8::try_from(month).expect("valid month")),
                optional_int(self.date_day, i).map(|day| u8::try_from(day).expect("valid day")),
            )
            .expect("valid date"),
            outcome: self.outcome.get(i).expect("required outcome"),
            speed: self.speed.get(i),
            rating_mode: self.rating_mode.get(i).expect("required rating_mode"),
            // Empty setups are treated like missing setups.
            setup: Some(self.setup.value(i))
                .filter(|bytes| self.setup.is_valid(i) && !bytes.is_empty()),
            event: self.event.as_ref().and_then(|event| event.get(i)),
            moves: self.moves.value(i),
        };
        (GameId::unpack(&mut self.game_id.value(i)), game)
    }
}

fn column<'a>(batch: &'a RecordBatch, name: &str) -> &'a dyn Array {
    batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("column {name}"))
        .as_ref()
}

#[inline]
fn optional_int(array: &Int32Array, i: usize) -> Option<i32> {
    array.is_valid(i).then(|| array.value(i))
}

#[cfg(test)]
mod tests {
    use parquet::file::writer::SerializedFileWriter;
    use shakmaty::KnownOutcome;

    use super::*;
    use crate::{
        packed::{
            EventInfo, GamePlayer, LaxDate, LichessGame, PackedMoves, Rating, RatingMode, Speed,
        },
        parquet::GameColumns,
    };

    fn game(title: Option<&str>, speed: Option<Speed>, event: Option<EventInfo>) -> LichessGame {
        LichessGame {
            players: ByColor {
                white: GamePlayer {
                    title: title.map(str::to_owned),
                    name: "white".to_owned(),
                    rating: Some(Rating::try_from(1500).unwrap()),
                },
                black: GamePlayer {
                    title: None,
                    name: "black".to_owned(),
                    rating: None,
                },
            },
            date: LaxDate::from_ymd(2020, Some(1), None).unwrap(),
            outcome: KnownOutcome::Draw,
            speed,
            rating_mode: RatingMode::Rated,
            setup: None,
            event,
            moves: PackedMoves(vec![0x12, 0x34]),
        }
    }

    #[test]
    fn test_rows_match_generic_reader() {
        let mut columns = GameColumns::default();
        let event = EventInfo {
            event: "Event".to_owned(),
            site: "Site".to_owned(),
            round: "1".to_owned(),
        };
        for (game_id, title, speed, event) in [
            ("abcdefgh", Some("GM"), Some(Speed::Blitz), None),
            ("12345678", None, None, Some(event)),
            ("ABCDEFGH", Some("FM"), Some(Speed::Classical), None),
        ] {
            columns.push(game_id.parse().unwrap(), game(title, speed, event));
        }

        let mut file = Vec::new();
        let mut writer = SerializedFileWriter::new(
            &mut file,
            GameColumns::schema(),
            Arc::new(GameColumns::properties().build()),
        )
        .unwrap();
        columns.write_row_group(&mut writer).unwrap();
        writer.close().unwrap();

        let file_len = file.len();
        let reader = GameReader::try_new(Bytes::from(file)).unwrap();
        assert_eq!(reader.num_row_groups(), 1);
        let batch = reader.read_games(0).unwrap();
        assert_eq!(batch.len(), 3);

        let expected: Vec<_> = columns.rows().collect();
        let actual: Vec<_> = batch.rows().collect();
        assert_eq!(actual, expected);

        let fetched = reader.fetch_row_group(0).unwrap();
        assert!(fetched.len() < file_len);
        let batch = reader.read_games_from(0, fetched).unwrap();
        let actual: Vec<_> = batch.rows().collect();
        assert_eq!(actual, expected);

        let game_ids: Vec<GameId> = columns.rows().map(|(game_id, _)| game_id).collect();
        assert_eq!(reader.read_game_ids(0).unwrap(), game_ids);
    }
}
