macro_rules! embedded {
    () => {
        include_str!("data.txt")
    };
}

pub const DATA: &str = embedded!();
