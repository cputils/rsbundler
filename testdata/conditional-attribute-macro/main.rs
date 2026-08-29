#[cfg_attr(transformed, custom::transform)]
mod dependency;

fn main() {
    #[cfg(not(transformed))]
    println!("{}", dependency::VALUE);
}
