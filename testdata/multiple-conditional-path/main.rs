#[cfg_attr(choice_a, path = "a.rs")]
#[cfg_attr(choice_b, path = "b.rs")]
mod selected;

fn main() {
    println!("{}", selected::VALUE);
}
