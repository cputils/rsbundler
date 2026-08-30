use std::cfg as selected;

const ALWAYS: &str = include_str!(concat!("values/", cfg!(all()), ".txt"));
const NEVER: &str = include_str!(concat!("values/", cfg!(any()), ".txt"));
const TAUTOLOGY: &str = include_str!(concat!(
    "values/",
    selected!(any(target_family = "wasm", not(target_family = "wasm"))),
    ".txt",
));
const CONTRADICTION: &str = include_str!(concat!(
    "values/",
    cfg!(all(target_family = "wasm", not(target_family = "wasm"))),
    ".txt",
));
const TARGET_DEPENDENT: &str =
    include_str!(concat!("values/", cfg!(target_family = "unix"), ".txt"));

fn main() {
    print!(
        "{}|{}|{}|{}|{}",
        ALWAYS.trim(),
        NEVER.trim(),
        TAUTOLOGY.trim(),
        CONTRADICTION.trim(),
        TARGET_DEPENDENT.trim()
    );
}
