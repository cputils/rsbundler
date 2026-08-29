#[path = "child.rs"]
mod renamed;

fn main() {
    println!("{}", renamed::VALUE);
}
