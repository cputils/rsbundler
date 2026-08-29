mod data;
mod math;
mod outer {
    pub mod inner;
}
mod r#type;

fn main() {
    println!(
        "{}|{}|{}|{}|{}|{}",
        math::twice(data::VALUE),
        data::child::NAME,
        math::nested::LABEL,
        outer::inner::value(),
        r#type::NAME,
        math::RAW
    );
}
