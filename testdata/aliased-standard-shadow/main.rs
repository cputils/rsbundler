mod std {
    macro_rules! include_str {
        ($path:literal) => {
            "custom alias"
        };
    }

    pub(crate) use include_str;
}

use std::include_str as embedded;

const VALUE: &str = embedded!("data.txt");

fn main() {
    println!("{VALUE}");
}
