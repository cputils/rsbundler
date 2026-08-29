macro_rules! source_line {
    ($value:expr) => {
        {
            let _ = $value;
            line!()
        }
    };
}

pub const LINE: u32 = source_line!(1);
