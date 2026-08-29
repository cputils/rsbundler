use std::file as source_file;

macro_rules! current_source {
    () => {
        source_file!()
    };
}

pub const SOURCE: &str = current_source!();
