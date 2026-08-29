#[cfg_attr(special, path = "kept-special.rs")]
mod kept; // no-bundle

#[cfg_attr(special, path = "forced-special.rs")]
mod forced; // bundle

fn main() {
    println!("{}|{}", kept::VALUE, forced::VALUE);
}
