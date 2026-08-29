include!("generated/items.rs");

const INCLUDED_VALUE: i32 = 2 * include!("generated/value.rs");

fn main() {
    println!("{}|{}", included::value(), INCLUDED_VALUE);
}
