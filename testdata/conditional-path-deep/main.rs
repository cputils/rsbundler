#[cfg_attr(alternate, path = "alternate/mod.rs")]
mod selected;

fn main() {
    println!("{}", selected::child::VALUE);
}
