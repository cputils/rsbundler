mod first {
    use std::include_str as embedded;

    pub const VALUE: &str = embedded!("data.txt");
}

mod second {
    macro_rules! embedded {
        ($path:literal) => {
            "custom"
        };
    }

    pub const VALUE: &str = embedded!("missing.txt");
}

mod child;

macro_rules! include_str {
    ($path:literal) => {
        "defined too late for child"
    };
}

fn main() {
    println!("{}|{}|{}", first::VALUE.trim(), second::VALUE, child::VALUE.trim());
}
