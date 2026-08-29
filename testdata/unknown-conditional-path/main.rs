/// Platform selected by cfg.
#[doc = include_str!("platform.md")]
#[cfg_attr(custom_platform, path = "linux.rs")]
pub(crate) mod platform;

fn main() {
    println!("{}", platform::VALUE);
}
