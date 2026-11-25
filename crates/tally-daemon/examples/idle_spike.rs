//! Throwaway probe: prints idle_ms once per second via the platform's default idle source.
use tally_daemon::presence::{default_platform, Platform};

fn main() {
    let mut p = default_platform();
    for _ in 0..60 {
        match p.idle_ms() {
            Ok(ms) => println!("idle_ms={ms}"),
            Err(e) => println!("error: {e}"),
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
