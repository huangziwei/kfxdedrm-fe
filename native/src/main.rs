//! Entry point for [`kfxdedrm_fe_native::app::run`].

use kfxdedrm_fe_native::install::selfupdate::VERSION_FLAG;

fn main() {
    // Probed by `bin/launch.sh` to pick the build that runs on this device:
    // the install carries one binary per float ABI and only one of them
    // starts here. This opens no framebuffer.
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
