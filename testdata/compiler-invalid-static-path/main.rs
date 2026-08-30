const PARENTHESIZED: &str = include_str!(("data.txt"));
const NESTED_PARENTHESIZED: &str = include_str!(concat!(("data"), ".txt"));
const INVALID_SUFFIX: &str = include_str!(concat!(1foo, ".txt"));
const INVALID_FLOAT_SUFFIX: &str = include_str!(concat!(1.0foo, ".txt"));
const INVALID_STRING_SUFFIX: &str = include_str!("data.txt"foo);
const INVALID_CHAR_SUFFIX: &str = include_str!(concat!('a'foo, ".txt"));

fn main() {
    println!(
        "{PARENTHESIZED}{NESTED_PARENTHESIZED}{INVALID_SUFFIX}{INVALID_FLOAT_SUFFIX}{INVALID_STRING_SUFFIX}{INVALID_CHAR_SUFFIX}"
    );
}
