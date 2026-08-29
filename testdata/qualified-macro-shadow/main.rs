mod std {
    macro_rules! include_str {
        ($path:literal) => {
            "custom qualified macro"
        };
    }

    pub(crate) use include_str;
}

const VALUE: &str = std::include_str!("missing.txt");

fn main() {
    println!("{VALUE}");
}
