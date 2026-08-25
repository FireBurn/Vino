//! Just listen on EP 0x84 for spontaneous traffic from the dock.
//! Useful for debugging — should be silent if the dock is quiescent
//! and needs the host to send something first.

use vino_driver::{profile::Placement, Dock, EP_OUT_CTRL, MAX_HEADS};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // This tool only ever reads the control endpoint, so it places whatever it finds as a device
    // with one connector on that endpoint: no video is sent, and naming a family's video endpoints
    // here would only be a guess about hardware nobody has identified yet.
    let dock = Dock::open(|_family, _product| {
        Some(Placement {
            name: "control-endpoint listener",
            video_endpoints: [EP_OUT_CTRL; MAX_HEADS],
            connectors: 1,
        })
    })?;
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
