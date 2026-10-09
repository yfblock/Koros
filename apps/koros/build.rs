use std::{env, io::Result};

fn main() -> Result<()> {
    gen_linker_script()
}

fn gen_linker_script() -> Result<()> {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("can't find target");
    let package = env::var("CARGO_PKG_NAME").expect("can't find package name");
    let fname = format!("linker_{}_{}.lds", package, arch);
    // Kernel load addresses.  koros runs without an MMU: riscv64/aarch64
    // link identity at QEMU's physical load address (no paging at all);
    // x86_64 long mode mandates paging, so it keeps the high-half link on
    // kor-arch's static boot tables (identity low 4 GiB + high window).
    let (output_arch, kernel_base) = if arch == "x86_64" {
        ("i386:x86-64", "0xffff800000200000")
    } else if arch.contains("riscv64") {
        ("riscv", "0x80200000")
    } else if arch.contains("aarch64") {
        ("aarch64", "0x40080000")
    } else {
        (arch.as_str(), "0")
    };
    let ld_content = std::fs::read_to_string("../../tools/lds/koros.lds")?;
    let ld_content = ld_content.replace("%ARCH%", output_arch);
    let ld_content = ld_content.replace("%KERNEL_BASE%", kernel_base);

    // The generated script is an intermediate artifact: keep it inside
    // OUT_DIR (always under target/), never in the source tree.
    let ld_path = std::path::PathBuf::from(env::var("OUT_DIR").unwrap()).join(&fname);
    std::fs::write(&ld_path, ld_content)?;
    println!("cargo:rustc-link-arg=-T{}", ld_path.display());
    println!("cargo:rerun-if-env-changed=CARGO_CFG_KERNEL_BASE");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../tools/lds/koros.lds");
    Ok(())
}
