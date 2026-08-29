const DATA: &str = include_str!(concat!(
    stringify!(assets),
    "/data",
    concat!(),
    ".txt"
));

fn main() {
    println!("{}", DATA.trim());
}
