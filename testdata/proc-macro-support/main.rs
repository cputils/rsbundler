#[fixture_macros::bundle_module(5)]
const MARKER: () = ();

#[derive(Debug, fixture_macros::Answer)]
struct Marker;

const PROC_VALUE: usize = fixture_macros::include_value!();

fn main() {
    println!(
        "{}",
        dependency::VALUE + ATTRIBUTE_VALUE + Marker::answer() + PROC_VALUE
    );
}
