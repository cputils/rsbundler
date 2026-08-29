mod automatic;

mod forced; // bundle

fn main() {
    println!("{}|{}", automatic::VALUE, forced::value());
}
