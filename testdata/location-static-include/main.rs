mod child;

fn main() {
    print!(
        "{}|{}|{}",
        child::SELF.contains("pub const SELF"),
        child::LINE.trim(),
        child::COLUMN.trim()
    );
}
