use std::io::Write;

use parquet::{
    column::reader::get_typed_column_reader,
    data_type::DataType,
    file::{reader::RowGroupReader, writer::SerializedRowGroupWriter},
};

pub struct OptionalColumn<T: DataType> {
    values: Vec<T::T>,
    def_levels: Vec<i16>,
}

impl<T: DataType> Default for OptionalColumn<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            def_levels: Vec::new(),
        }
    }
}

impl<T: DataType> OptionalColumn<T> {
    pub fn is_empty(&self) -> bool {
        self.def_levels.is_empty()
    }

    pub fn len(&self) -> usize {
        self.def_levels.len()
    }

    pub fn push(&mut self, value: Option<T::T>) {
        match value {
            Some(value) => {
                self.values.push(value);
                self.def_levels.push(1);
            }
            None => self.def_levels.push(0),
        }
    }

    pub fn clear(&mut self) {
        self.values.clear();
        self.def_levels.clear();
    }

    pub fn reserve(&mut self, additional_rows: usize) {
        self.values.reserve(additional_rows);
        self.def_levels.reserve(additional_rows);
    }

    pub fn append(&mut self, other: &mut Self) {
        self.values.append(&mut other.values);
        self.def_levels.append(&mut other.def_levels);
    }

    pub fn iter(&self) -> impl Iterator<Item = Option<&T::T>> {
        let mut values = self.values.iter();
        self.def_levels
            .iter()
            .map(move |&def_level| if def_level > 0 { values.next() } else { None })
    }

    pub fn scatter(&self, partitions: &[Option<usize>], targets: &mut [&mut Self]) {
        for (value, &partition) in self.iter().zip(partitions) {
            if let Some(partition) = partition {
                targets[partition].push(value.cloned());
            }
        }
    }

    pub fn write_column<W>(
        &self,
        writer: &mut SerializedRowGroupWriter<'_, W>,
    ) -> parquet::errors::Result<()>
    where
        W: Write + Send,
    {
        let mut column_writer = writer.next_column()?.expect("column");
        column_writer
            .typed::<T>()
            .write_batch(&self.values, Some(&self.def_levels), None)?;
        column_writer.close()
    }

    pub fn read_column(
        &mut self,
        reader: &dyn RowGroupReader,
        column_index: usize,
    ) -> parquet::errors::Result<()> {
        let num_rows = reader.metadata().num_rows() as usize;

        self.clear();
        self.reserve(num_rows);

        let mut column_reader =
            get_typed_column_reader::<T>(reader.get_column_reader(column_index)?);

        let mut records = 0;
        while records < num_rows {
            let (read, _, _) = column_reader.read_records(
                num_rows - records,
                Some(&mut self.def_levels),
                None,
                &mut self.values,
            )?;
            if read == 0 {
                break;
            }
            records += read;
        }

        Ok(())
    }
}
