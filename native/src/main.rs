//! Entry point for [`kfxdedrm_fe_native::app::run`].

use kfxdedrm_fe_native::install::selfupdate::VERSION_FLAG;

fn main() {
    // `bin/launch.sh` runs this over each name in `bin/`. It opens no
    // framebuffer.
    if std::env::args().skip(1).any(|a| a == VERSION_FLAG) {
        println!("kfxdedrm-fe {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Err(e) = kfxdedrm_fe_native::app::run() {
        // `launch.sh` redirects stderr to /mnt/us/logs/kfxdedrm-fe.log.
        eprintln!("fatal: {e:#}");
        std::process::exit(1);
    }
}
