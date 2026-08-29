#[cfg_attr(alternate, path = "alternate")]
mod container {
    pub mod child;
}

fn main() {
    println!("{}", container::child::VALUE);
}
