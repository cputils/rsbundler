const FILE: &str = "data.txt";
const DATA: &str = include_str!(FILE); // bundle

fn main() {
    println!("{DATA}");
}
