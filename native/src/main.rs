//! Entry point for [`kfxdedrm_fe_native::app::run`].

fn main() {
    if let Err(e) = kfxdedrm_fe_native::app::run() {
        // `launch.sh` redirects stderr to /mnt/us/logs/kfxdedrm-fe.log. The
        // window is gone by this point.
        eprintln!("fatal: {e:#}");
        std::process::exit(1);
    }
}
