macro_rules! declare_module {
    () => {
        mod child;
    };
}

declare_module!();

pub fn value() -> &'static str {
    child::VALUE
}
