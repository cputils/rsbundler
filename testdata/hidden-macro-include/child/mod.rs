pub fn value() -> &'static str {
    identity!(include_str!("data.txt"))
}
