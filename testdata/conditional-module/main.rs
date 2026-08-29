#[cfg(any())]
mod unavailable;

#[cfg(all())]
mod available;

#[cfg(any())]
const UNAVAILABLE_TEXT: &str = include_str!(MISSING_PATH);

fn main() {
    println!("{}", available::VALUE);
}
