//! Compile the small AppKit host. Database/application state stays in Rust.
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=native/NativeShell.swift");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        arch => panic!("Unsupported macOS architecture: {arch}"),
    };
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-library",
            "-static",
            "-O",
            "-module-name",
            "DBXNativeShell",
            "-target",
        ])
        .arg(format!("{arch}-apple-macosx11.0"))
        .arg("native/NativeShell.swift")
        .arg("-o")
        .arg(out.join("libDBXNativeShell.a"))
        .status()
        .expect("Xcode command line tools are required to build DBX on macOS");
    assert!(status.success(), "Failed to compile the AppKit host");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    let swiftc = Command::new("xcrun")
        .args(["--find", "swiftc"])
        .output()
        .expect("locate Swift toolchain");
    let swiftc = PathBuf::from(String::from_utf8(swiftc.stdout).unwrap().trim());
    let runtime = swiftc
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib/swift/macosx");
    println!("cargo:rustc-link-search=native={}", runtime.display());
    println!("cargo:rustc-link-lib=static=DBXNativeShell");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
