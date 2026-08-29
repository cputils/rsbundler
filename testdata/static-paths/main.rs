const DATA: &str = include_str!(concat!(env!("ASSET_DIR"), "/", 1, true, 'x', ".txt"));

fn main() {
    println!("{}", DATA.trim());
}
