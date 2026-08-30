use std::file as source_file;

pub const SELF: &str = include_str!(source_file!());
pub const LINE: &str = include_str!(concat!("line/", line!(), ".txt"));
pub const COLUMN: &str = include_str!(concat!(
    "column/",
    core::column!(),
    ".txt",
));
