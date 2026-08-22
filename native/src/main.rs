//! Entry point for [`kfxdedrm_fe_native::app::run`].

/// Probed by `bin/launch.sh` to pick the build that runs on this device.
///
/// The install carries one binary per float ABI and only one of them starts
/// here; this is the invocation that says which, and it opens no framebuffer.
const VERSION_FLAG: &str = "--version";

fn main() {
    if std::env::args().skip(1).any(|a| a == VERSION_FLAG) {
        println!("kfxdedrm-fe {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(e) = kfxdedrm_fe_native::app::run() {
        // `launch.sh` redirects stderr to /mnt/us/logs/kfxdedrm-fe.log. The
        // window is gone by this point.
        eprintln!("fatal: {e:#}");
        std::process::exit(1);
    }
}
