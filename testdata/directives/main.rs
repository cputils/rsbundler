mod automatic;

// no-bundle
mod kept;

mod forced; // bundle

const EXPANDED: &str = include_str!("expanded.txt");
const KEPT: &str = include_str!("kept.txt"); // no-bundle

fn main() {
    println!("{}|{}|{}|{}", automatic::VALUE, kept::VALUE, forced::VALUE, EXPANDED.trim());
    assert_eq!(KEPT.trim(), "kept include");
}
