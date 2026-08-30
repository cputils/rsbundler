macro_rules! identity {
    ($value:expr) => {
        $value
    };
}

mod child;

fn main() {
    println!("{}", child::value().trim());
}
