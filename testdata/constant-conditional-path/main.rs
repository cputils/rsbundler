#[cfg_attr(any(), path = "missing.rs")]
mod fallback;

#[cfg_attr(all(), path = "chosen.rs")]
mod selected;

fn main() {
    println!("{}|{}", fallback::VALUE, selected::VALUE);
}
