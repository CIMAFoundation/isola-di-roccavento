fn main() {
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32");
    let mut build = cc::Build::new();
    if wasm {
        // wasm32-unknown-unknown has no libc: freestanding, with the small
        // headers in wasm/shim and the stubs in wasm/libc.c. Apple's clang
        // cannot target wasm; use an LLVM one (CC_wasm32_unknown_unknown wins).
        // Homebrew may install it versioned only (`llvm@22`, 2026-11-01): the
        // plain `llvm` first, then the `llvm@N`, newest first.
        if std::env::var_os("CC_wasm32_unknown_unknown").is_none() {
            for dir in llvm_dirs() {
                let clang = dir.join("clang");
                if clang.exists() {
                    build.compiler(clang);
                    if std::env::var_os("AR_wasm32_unknown_unknown").is_none() {
                        build.archiver(dir.join("llvm-ar"));
                    }
                    break;
                }
            }
        } else if std::env::var_os("AR_wasm32_unknown_unknown").is_none() {
            // the llvm-ar next to the clang given (Apple's ar cannot index wasm objects)
            let cc = std::path::PathBuf::from(std::env::var_os("CC_wasm32_unknown_unknown").unwrap());
            if let Some(ar) = cc.parent().map(|d| d.join("llvm-ar")).filter(|p| p.exists()) {
                build.archiver(ar);
            }
        }
        build
            .flag("-ffreestanding")
            .include("wasm/shim")
            .include("vendor/src/decomp")
            .file("wasm/libc.c");
        println!("cargo:rerun-if-changed=wasm");
    }
    build
        .include("vendor/src/decomp/include")
        .define("SM64_LIB_EXPORT", None)
        .define("GBI_FLOATS", None)
        .define("VERSION_US", None)
        .define("NO_SEGMENTED_MEMORY", None)
        .flag_if_supported("-fno-strict-aliasing")
        .warnings(false);
    let dirs = [
        "",
        "decomp",
        "decomp/engine",
        "decomp/include/PR",
        "decomp/game",
        "decomp/pc",
        "decomp/pc/audio",
        "decomp/mario",
        "decomp/tools",
        "decomp/audio",
    ];
    let mut files = Vec::new();
    for dir in dirs {
        let path = std::path::Path::new("vendor/src").join(dir);
        if !path.exists() {
            continue;
        }
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "c") {
                files.push(path);
            }
        }
    }
    if wasm {
        // ROM-extraction tooling that needs a filesystem; wasm/libc.c has the
        // one helper (`read_f32_be`) the rest uses.
        files.retain(|f| f.file_name().is_some_and(|n| n != "utils.c"));
    }
    files.sort();
    build.files(files).compile("sm64");
    println!("cargo:rerun-if-changed=vendor/src");
    if std::env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("unix") {
        println!("cargo:rustc-link-lib=m");
    }
}

/// The `bin` of each LLVM Homebrew may have installed: `llvm`, then
/// `llvm@N` from the highest N.
fn llvm_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    for root in ["/opt/homebrew/opt", "/usr/local/opt"] {
        let root = std::path::Path::new(root);
        dirs.push(root.join("llvm/bin"));
        let mut versioned: Vec<(u32, std::path::PathBuf)> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let n = name.strip_prefix("llvm@")?.parse().ok()?;
                Some((n, e.path().join("bin")))
            })
            .collect();
        versioned.sort_by(|a, b| b.0.cmp(&a.0));
        dirs.extend(versioned.into_iter().map(|(_, d)| d));
    }
    dirs
}
