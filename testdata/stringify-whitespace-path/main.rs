const VALUE: &str = include_str!(stringify!(a    +b.txt));

fn main() {
    println!("{}", VALUE.trim());
}
