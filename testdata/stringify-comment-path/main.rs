const DATA: &str = include_str!(stringify!(/* keep Rust's trivia semantics */ assets/data.txt));

fn main() {
    println!("{}", DATA.trim());
}
