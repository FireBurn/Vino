//! `chimera-prove` — offline byte-exact proof of the kernel CP path.
//!
//! Runs both equivalence proofs in [`vino_chimera::prove`] against an embedded
//! real DLM session and exits non-zero if the literal kernel `cp.rs` ever
//! diverges from what DLM put on the wire.

fn main() -> std::process::ExitCode {
    let r = vino_chimera::prove::run();
    println!(
        "\nsummary: seal {}/{} byte-identical ({} skipped), builders {}/{} ok\n{}",
        r.seal_ok,
        r.seal_total,
        r.seal_skipped,
        r.builder_documented,
        r.builder_total,
        if r.passed() {
            "RESULT: PASS — the in-kernel CP encryption + post-msg0 builders are proven \
             byte-exact against DLM."
        } else {
            "RESULT: FAIL — divergence from DLM detected."
        }
    );
    if r.passed() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
