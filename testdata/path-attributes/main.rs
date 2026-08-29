#[path = "platform/implementation.rs"]
mod imp;

#[path = "renamed"]
mod container {
    #[path = "child.rs"]
    pub mod child;
}

fn main() {
    println!(
        "{}|{}|{}",
        imp::nested::value(),
        imp::default_child::value(),
        container::child::value()
    );
}

