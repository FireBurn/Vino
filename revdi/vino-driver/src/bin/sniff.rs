//! Just listen on EP 0x84 for spontaneous traffic from the dock.
//! Useful for debugging — should be silent if the dock is quiescent
//! and needs the host to send something first.

use vino_driver::Dock;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dock = Dock::open()?;
    println!("opened. listening 5s on EP 0x84...");
    for i in 0..5 {
        match dock.recv_frame_raw(4096) {
            Ok(b) => println!(
                "  [{i}] got {} bytes: {:02x?}",
                b.len(),
                &b[..b.len().min(36)]
            ),
            Err(e) => println!("  [{i}] {e}"),
        }
    }
    Ok(())
}
