use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Sign a payload with HMAC-SHA256. Returns hex-encoded signature.
/// Matches the TypeScript implementation exactly.
pub fn sign_payload(payload: &str, secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can accept any key length");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Verify a HMAC-SHA256 signature using constant-time comparison.
pub fn verify_signature(payload: &str, signature: &str, secret: &str) -> bool {
    let expected = sign_payload(payload, secret);

    if expected.len() != signature.len() {
        return false;
    }

    // Constant-time comparison to prevent timing attacks
    let expected_bytes = expected.as_bytes();
    let sig_bytes = signature.as_bytes();
    let mut diff: u8 = 0;
    for (a, b) in expected_bytes.iter().zip(sig_bytes.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let payload = r#"{"test":"value"}"#;
        let secret = "test-secret";
        let sig = sign_payload(payload, secret);
        assert!(verify_signature(payload, &sig, secret));
        assert!(!verify_signature(payload, "wrong", secret));
        assert!(!verify_signature("different", &sig, secret));
    }

    #[test]
    fn matches_ts_implementation() {
        // Known vector: computed with the TypeScript implementation
        let payload = "hello";
        let secret = "secret";
        let sig = sign_payload(payload, secret);
        // HMAC-SHA256("secret", "hello") = 88aab3ede8d3adf94d26ab90d3bafd4a2083070c3bcce9c014ee04a443847c0b
        assert_eq!(sig, "88aab3ede8d3adf94d26ab90d3bafd4a2083070c3bcce9c014ee04a443847c0b");
    }
}
