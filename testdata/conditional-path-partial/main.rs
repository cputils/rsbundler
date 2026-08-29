#[cfg_attr(generated, path = "generated.rs")]
mod selected;

fn main() {
    #[cfg(not(generated))]
    println!("{}", selected::VALUE);
}
