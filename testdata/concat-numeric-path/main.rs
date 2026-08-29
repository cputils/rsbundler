const DATA: &str = include_str!(concat!(
    "assets/",
    1_000u32,
    0xff_u8,
    1.20e+3f64,
    -2i16,
    -1.5f32,
    ".txt"
));

fn main() {
    println!("{}", DATA.trim());
}
