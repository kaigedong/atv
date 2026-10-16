use atv_auth::{LiveResolver, ResolverConfig};
fn main() {
    let mut args = std::env::args().skip(1);
    let pid = args.next().unwrap_or_else(|| "600001859".into());
    let stream = args.next().unwrap_or_else(|| "2024078201".into());
    let quality = args.next().unwrap_or_else(|| "hd".into());
    let resolver = LiveResolver::new(ResolverConfig::default()).expect("initialize");
    match resolver.resolve_quality(&pid, &stream, &quality, 1) {
        Ok(info) => println!(
            "resolved=true defn={} previewSeconds={} urlPresent={}",
            info.definition,
            info.preview_seconds,
            !info.url().is_empty()
        ),
        Err(error) => {
            eprintln!("resolved=false error={error}");
            std::process::exit(1)
        }
    }
}
