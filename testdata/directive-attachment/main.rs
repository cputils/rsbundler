mod first; mod second; // no-bundle

// no-bundle
mod third; mod fourth;

const A: &str = include_str!("a.txt"); const B: &str = include_str!("b.txt"); // no-bundle

fn main() {
    println!(
        "{}|{}|{}|{}|{}|{}",
        first::VALUE,
        second::VALUE,
        third::VALUE,
        fourth::VALUE,
        A.trim(),
        B.trim()
    );
}
