//! Build script: embed the Windows icon and version resource into
//! iris.exe, with the version taken from Cargo.toml. A no-op on every
//! other platform.

fn main() {
    // Without a rerun-if-changed line cargo reruns this script whenever
    // any file of the package changes.
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    {
        // Re-run only when the resource or icon changes; a version bump
        // changes the package and re-runs every build script.
        println!("cargo:rerun-if-changed=packaging/windows/iris.rc");
        println!("cargo:rerun-if-changed=packaging/icons/iris.ico");
        let var = |name: &str| std::env::var(name).expect("cargo sets the package version");
        let quad = format!(
            "IRIS_VERSION_QUAD={},{},{},0",
            var("CARGO_PKG_VERSION_MAJOR"),
            var("CARGO_PKG_VERSION_MINOR"),
            var("CARGO_PKG_VERSION_PATCH"),
        );
        let text = format!("IRIS_VERSION=\"{}\"", var("CARGO_PKG_VERSION"));
        embed_resource::compile("packaging/windows/iris.rc", [quad, text])
            .manifest_optional()
            .expect("compile iris.rc into iris.exe");
    }
}
