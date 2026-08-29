macro_rules! include_str {
    ($path:literal) => {
        "custom macro"
    };
}

macro_rules! concat {
    ($ignored:literal) => {
        "data.txt"
    };
}

use std::include_str as embedded;

mod child;

const CUSTOM_INCLUDE: &str = include_str!("missing.txt");
const CUSTOM_CONCAT: &str = std::include_str!(concat!("ignored.txt"));
const QUALIFIED: &str = std::include_str!("data.txt");
const ALIASED: &str = embedded!("data.txt");

fn main() {
    println!("{CUSTOM_INCLUDE}|{}|{}|{}|{}", CUSTOM_CONCAT.trim(), QUALIFIED.trim(), ALIASED.trim(), child::VALUE);
}
