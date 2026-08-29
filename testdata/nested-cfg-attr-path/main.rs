#[cfg_attr(
    outer,
    cfg_attr(feature = "inner", path = "nested.rs"),
    allow(dead_code)
)]
mod selected;

fn main() {
    println!("{}", selected::VALUE);
}
