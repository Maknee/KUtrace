fn main() {
    println!("cargo:rerun-if-changed=src/probes.c");
    cc::Build::new()
        .file("src/probes.c")
        .flag_if_supported("-fno-omit-frame-pointer")
        .compile("kutrace_usdt_probes");
}
