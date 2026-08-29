mod generated;
mod hidden_include;

fn main() {
    println!("{}|{}", generated::value(), hidden_include::DATA.trim());
}
