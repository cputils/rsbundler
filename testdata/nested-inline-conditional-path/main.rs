#[cfg_attr(outer_alt, path = "outer-alt")]
mod outer {
    #[cfg_attr(inner_alt, path = "inner-alt")]
    pub mod inner {
        pub mod leaf;
    }
}

fn main() {
    println!("{}", outer::inner::leaf::VALUE);
}
