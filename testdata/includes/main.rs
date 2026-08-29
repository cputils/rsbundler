const MESSAGE: &str = include_str!(concat!("assets/", "message.txt"));
static BYTES: &[u8; 5] = include_bytes!("assets/bytes.bin");
static EMPTY: &[u8; 0] = include_bytes!("assets/empty.bin");

include!("generated/items.rs");

fn main() {
    println!("{}|{}|{}|{}", MESSAGE.trim(), BYTES[1], EMPTY.len(), generated_value());
}
