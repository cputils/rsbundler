const PATH: &str = "data.txt";

macro_rules! include_str {
    ($path:expr) => {
        "nested dynamic"
    };
}

pub const DATA: &str = include_str!(PATH);
