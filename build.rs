//! Build script: embed the Windows icon + version resource into
//! iris.exe. A no-op on every other platform.

fn main() {
    #[cfg(windows)]
    {
        // Re-run only when the resource or icon changes.
        println!("cargo:rerun-if-changed=packaging/windows/iris.rc");
        println!("cargo:rerun-if-changed=packaging/icons/iris.ico");
        embed_resource::compile("packaging/windows/iris.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("compile iris.rc into iris.exe");
    }
}
