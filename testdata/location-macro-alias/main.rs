use std::{column as source_column, file as source_file, line as source_line};

const SOURCE: &str = source_file!();
const LINE: u32 = source_line!();
const COLUMN: u32 = source_column!();

fn main() {
    println!("{SOURCE}:{LINE}:{COLUMN}");
}
