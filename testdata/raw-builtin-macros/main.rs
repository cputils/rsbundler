mod child;

fn main() {
    println!("{}:{}", child::LINE, child::DATA.trim());
}
