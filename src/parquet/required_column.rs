use std::io::Write;

use parquet::{
    column::reader::get_typed_column_reader,
    data_type::DataType,
    file::{reader::RowGroupReader, writer::SerializedRowGroupWriter},
};

pub struct RequiredColumn<T: DataType> {
    values: Vec<T::T>,
}

impl<T: DataType> Default for RequiredColumn<T> {
    fn default() -> Self {
        Self { values: Vec::new() }
    }
}

impl<T: DataType> RequiredColumn<T> {
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn push(&mut self, value: T::T) {
        self.values.push(value);
    }

    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub fn reserve(&mut self, additional_rows: usize) {
        self.values.reserve(additional_rows);
    }

    pub fn append(&mut self, other: &mut Self) {
        self.values.append(&mut other.values);
    }

    pub fn iter(&self) -> impl Iterator<Item = &T::T> {
        self.values.iter()
    }

    pub fn scatter(&self, partitions: &[Option<usize>], targets: &mut [&mut Self]) {
        for (value, &partition) in self.values.iter().zip(partitions) {
            if let Some(partition) = partition {
                targets[partition].push(value.clone());
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
            .write_batch(&self.values, None, None)?;
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
            let (read, _, _) =
                column_reader.read_records(num_rows - records, None, None, &mut self.values)?;
            if read == 0 {
                break;
            }
            records += read;
        }

        Ok(())
    }
}
