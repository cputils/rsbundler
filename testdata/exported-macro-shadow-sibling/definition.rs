#[macro_export]
macro_rules! include_str {
    ($path:literal) => {
        "sibling exported macro"
    };
}
