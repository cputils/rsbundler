mod unavailable;

const PATH: &str = "unavailable.txt";
const DATA: &str = include_str!(PATH);

fn main() {
    println!("{DATA}");
}
