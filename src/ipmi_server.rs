use serde::Serialize;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_bounds_and_handler_offset() {
        assert!(decode_server_descriptor(&[0; 48]).is_err());
        let mut bytes = [0; 56];
        assert!(decode_server_descriptor(&bytes).is_err());
        bytes[..8].copy_from_slice(&56u64.to_le_bytes());
        bytes[48..56].copy_from_slice(&0x12345678u64.to_le_bytes());
        assert_eq!(
            decode_server_descriptor(&bytes).unwrap().event_handler,
            0x12345678
        );
    }
}
#[derive(Debug, Serialize)]
pub struct ServerDescriptor {
    pub declared_bytes: u64,
    pub fields_08_14: [u32; 4],
    pub multi_thread: u32,
    pub field_1c: u32,
    pub field_20: u64,
    pub user_data: u64,
    pub event_handler: u64,
}
pub fn decode_server_descriptor(bytes: &[u8]) -> Result<ServerDescriptor, String> {
    if bytes.len() != 56 {
        return Err("Expected 56 server descriptor bytes".into());
    }
    let q = |p| u64::from_le_bytes(bytes[p..p + 8].try_into().unwrap());
    let d = |p| u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
    if q(0) != 56 {
        return Err("Server descriptor size field differs from the measured ABI".into());
    }
    Ok(ServerDescriptor {
        declared_bytes: q(0),
        fields_08_14: [d(8), d(12), d(16), d(20)],
        multi_thread: d(24),
        field_1c: d(28),
        field_20: q(32),
        user_data: q(40),
        event_handler: q(48),
    })
}
