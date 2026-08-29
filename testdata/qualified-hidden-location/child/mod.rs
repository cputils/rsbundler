macro_rules! source_file {
    () => {
        ::std::file!()
    };
}

pub const SOURCE: &str = source_file!();
