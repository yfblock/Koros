use std::{env, io::Result};

fn main() -> Result<()> {
    gen_linker_script()
}

fn gen_linker_script() -> Result<()> {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("can't find target");
    let fname = format!("linker_{}.lds", arch);
    // Kernel load addresses — must match QEMU's physical load address
    // for each architecture so that PC-relative code works correctly.
    let (output_arch, kernel_base) = if arch == "x86_64" {
        ("i386:x86-64", "0xffff800000200000")
    } else if arch.contains("riscv64") {
        ("riscv", "0xffffffc080200000")
    } else if arch.contains("aarch64") {
        ("aarch64", "0xffff000040080000")
    } else if arch.contains("loongarch64") {
        ("loongarch64", "0x9000000080000000")
    } else {
        (arch.as_str(), "0")
    };
    let ld_content = std::fs::read_to_string("linker.lds")?;
    let ld_content = ld_content.replace("%ARCH%", output_arch);
    let ld_content = ld_content.replace("%KERNEL_BASE%", kernel_base);

    let out_dir = env::var("OUT_DIR").unwrap();
    // Go from OUT_DIR (target/<target>/release/build/<crate>-<hash>/out)
    // up to the target directory.
    let target_dir: std::path::PathBuf = [&out_dir, "..", "..", "..", "..", ".."]
        .iter()
        .collect();
    let ld_path = target_dir.join(&fname);
    std::fs::write(&ld_path, ld_content)?;
    println!("cargo:rustc-link-arg=-T{}", ld_path.display());
    println!("cargo:rerun-if-env-changed=CARGO_CFG_KERNEL_BASE");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=linker.lds");
    Ok(())
}
