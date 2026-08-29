#[cfg_attr(selected, path = "selected.rs")]
#[cfg_attr(not(selected), path = "fallback.rs")]
mod implementation;

fn main() {
    println!("{}", implementation::VALUE);
}
