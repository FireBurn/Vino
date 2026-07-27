# HDCP 2.2 session

Vino reuses the kernel's HDCP 2.2 message identifiers and crypto primitives.
Only DisplayLink-specific transport framing and message IDs absent from the DRM
header remain local.

## Observed exchange

```text
AKE_Init
AKE_Transmitter_Info
AKE_Send_Cert + AKE_Receiver_Info
AKE_No_Stored_km
AKE_Send_Rrx
AKE_Send_H_prime
AKE_Send_Pairing_Info
LC_Init
LC_Send_L_prime
SKE_Send_Eks
RepeaterAuth_Send_ReceiverID_List
RepeaterAuth_Send_Ack
RepeaterAuth_Stream_Manage
Receiver_Auth_Status
RepeaterAuth_Stream_Ready
```

The D6000 path uses `AKE_No_Stored_km` and RSA-OAEP with SHA-256. H', L', V',
and M' are verified locally. A per-head restatement establishes the downstream
streams after the shared AKE.

## Verified derivations

For `rtx`, `rrx`, master key `km`, locality nonce `rn`, and session key `ks`:

```text
dkey_n = AES-128(km with low 8 bytes XOR rn,
                 rtx || (rrx with byte 7 XOR n))
kd     = dkey_0 || dkey_1
H'     = HMAC-SHA256(kd, rtx with repeater bit mixed into byte 7)
L'     = HMAC-SHA256(kd with its low 8 bytes XOR rrx, rn)
```

Here “low 8 bytes” means the least-significant bytes: `km[8..16]` or
`kd[24..32]`. The counter is applied to the last byte of the 16-byte
`rtx || rrx` block.

Repeater authentication computes the full 256-bit V. The receiver supplies the
MSB half as V'; the host sends the LSB half in `RepeaterAuth_Send_Ack`.
Echoing V' as the acknowledgement is incorrect.

SKE wraps `ks` with `dkey_2`, the locality nonce, and `rrx`. Content may not be
sent until the repeater stream-management exchange completes successfully.

## Kernel integration

- standard message IDs: `kernel::drm::display::hdcp::MessageId`;
- AES and AES-CMAC: kernel crypto library wrappers;
- SHA-256 and HMAC-SHA256: kernel crypto library wrappers;
- RSA public operation: the `lib/crypto` primitive introduced separately;
- random session material: the kernel random interface.

The protocol code does not expose keys through a userspace ABI. Known-answer
tests and captured transcripts exercise the implementation without sharing an
encoder and decoder that could repeat the same mistake.

