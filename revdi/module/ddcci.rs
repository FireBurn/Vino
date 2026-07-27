// SPDX-License-Identifier: GPL-2.0
//
// DDC/CI over a virtual I2C adapter. Monitor-control tools (ddcutil, the desktop brightness
// slider, ...) issue I2C transfers to the DDC/CI address on the adapter EVDI registers; those are
// forwarded to the DisplayLinkManager client as DDCCI_DATA events, and the client's reply comes
// back through the DDCCI_RESPONSE ioctl (see `EvdiDrmData::ddcci_transfer`/`ddcci_respond`).

use kernel::{i2c, prelude::*, sync::aref::ARef};

use crate::kms::{EvdiDrmData, EvdiDrmDevice, DDCCI_ADDRESS};

/// The EVDI DDC/CI I2C bus controller.
pub(crate) struct EvdiI2c;

impl i2c::BusController for EvdiI2c {
    type Context = ARef<EvdiDrmDevice>;

    fn master_xfer(dev: &ARef<EvdiDrmDevice>, msgs: &mut [i2c::Msg]) -> Result<usize> {
        let ddev: &EvdiDrmDevice = dev;
        let data: &EvdiDrmData = ddev;
        let mut transferred = 0usize;
        for msg in msgs.iter_mut() {
            // Only DDC/CI traffic is forwarded to the client; ignore anything else on the bus.
            if msg.addr() != DDCCI_ADDRESS {
                continue;
            }
            let addr = msg.addr();
            let flags = msg.flags();
            if data.ddcci_transfer(addr, flags, msg.buffer()).is_ok() {
                transferred += 1;
            }
        }
        Ok(transferred)
    }

    fn functionality(_dev: &ARef<EvdiDrmDevice>) -> u32 {
        i2c::FUNC_I2C
    }
}
