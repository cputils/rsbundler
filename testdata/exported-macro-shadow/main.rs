#[macro_use]
mod macros;
mod child;

fn main() {
    println!("{}", child::VALUE);
}
