const GENERATED_TEXT: &str = include_str!("value.txt");

fn generated_value() -> &'static str {
    GENERATED_TEXT.trim()
}

