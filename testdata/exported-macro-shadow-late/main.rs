const VALUE: &str = include_str!("data.txt");

#[macro_export]
macro_rules! include_str {
    ($path:literal) => {
        "late exported macro"
    };
}

fn main() {
    println!("{VALUE}");
}
