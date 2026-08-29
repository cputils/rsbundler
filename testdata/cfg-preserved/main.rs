#[cfg(feature = "alpha")]
mod alpha;

#[cfg(not(feature = "alpha"))]
mod fallback;

#[cfg(feature = "alpha")]
const TEXT: &str = include_str!("alpha.txt");

#[cfg(not(feature = "alpha"))]
const TEXT: &str = include_str!("fallback.txt");

fn main() {
    #[cfg(feature = "alpha")]
    println!("{}|{}", alpha::VALUE, TEXT.trim());

    #[cfg(not(feature = "alpha"))]
    println!("{}|{}", fallback::VALUE, TEXT.trim());
}
