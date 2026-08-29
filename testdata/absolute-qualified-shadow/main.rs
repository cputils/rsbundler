#![no_std]

extern crate self as std;

#[macro_export]
macro_rules! include_str {
    ($path:literal) => {
        "custom absolute macro"
    };
}

const DATA: &str = ::std::include_str!("missing.txt");

fn main() {
    let _ = DATA;
}
