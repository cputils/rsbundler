mod child;

const ENTRY_FILE: &str = file!();
const ENTRY_LINE: u32 = line!();
/* あ */ const ENTRY_COLUMN: u32 = column!();
const INCLUDED: (&str, u32, u32) = include!("values.rs");

fn main() {
    println!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        ENTRY_FILE,
        ENTRY_LINE,
        ENTRY_COLUMN,
        child::FILE,
        child::LINE,
        child::COLUMN,
        INCLUDED.0,
        INCLUDED.1,
        INCLUDED.2
    );
}
