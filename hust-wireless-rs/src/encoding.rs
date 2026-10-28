/// Percent-encode like JavaScript's `encodeURIComponent` with the safe set
/// used by the ePortal page (`~!*'()-._`).
pub fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'!'
            | b'*'
            | b'\''
            | b'('
            | b')' => encoded.push(*byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

pub fn double_encode(value: &str) -> String {
    encode_component(&encode_component(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_portal_safe_characters() {
        assert_eq!(encode_component("A-z_0.9~!*'()"), "A-z_0.9~!*'()");
    }

    #[test]
    fn encodes_reserved_characters() {
        assert_eq!(encode_component("user@hust"), "user%40hust");
        assert_eq!(encode_component("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(encode_component(" "), "%20");
    }

    #[test]
    fn double_encodes() {
        assert_eq!(
            double_encode("mac=12&x=a%20b"),
            "mac%253D12%2526x%253Da%252520b"
        );
    }
}
