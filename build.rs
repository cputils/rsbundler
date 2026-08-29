use std::env;

fn main() {
    let mut atoms = env::vars()
        .filter_map(|(key, value)| {
            let name = key.strip_prefix("CARGO_CFG_")?.to_ascii_lowercase();
            Some(if value.is_empty() {
                vec![name]
            } else {
                value
                    .split(',')
                    .map(|value| format!("{name}={value}"))
                    .collect()
            })
        })
        .flatten()
        .collect::<Vec<_>>();
    atoms.sort();
    atoms.dedup();
    println!("cargo:rustc-env=RSBUNDLER_DEFAULT_CFG={}", atoms.join("|"));
}
