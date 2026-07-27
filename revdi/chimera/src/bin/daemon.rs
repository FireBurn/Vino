// SPDX-License-Identifier: GPL-2.0-or-later

fn main() {
    if let Err(error) = vino_chimera::daemon::run() {
        eprintln!("chimera: {error}");
        std::process::exit(1);
    }
}
