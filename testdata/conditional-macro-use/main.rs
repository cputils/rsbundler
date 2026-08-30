#[cfg_attr(all(), macro_use)]
mod macros;

const VALUE: &str = include_str!("data.txt");

fn main() {
    println!("{VALUE}");
}
