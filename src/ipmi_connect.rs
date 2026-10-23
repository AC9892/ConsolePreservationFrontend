//! Bounded decoder for the pinned native IPMI connection request.
use serde::Serialize;

/// Structural operation-0x320 layout corroborated by the pinned native wrapper.
#[derive(Debug, Serialize)]
pub struct SyncCallRequest {
    pub method: u32,
    pub input_count: u32,
    pub output_count: u32,
    pub inputs: u64,
    pub outputs: u64,
    pub method_result: u64,
    pub flags: u32,
}
pub fn decode_sync_call(bytes: &[u8]) -> Result<SyncCallRequest, String> {
    if bytes.len() != 48 {
        return Err("Expected 48 synchronous-call argument bytes".into());
    }
    let word32 = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let word64 = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let request = SyncCallRequest {
        method: word32(0),
        input_count: word32(4),
        output_count: word32(8),
        inputs: word64(16),
        outputs: word64(24),
        method_result: word64(32),
        flags: word32(40),
    };
    // Harness bounds, not a claim about hardware maximums. Padding is ignored.
    if request.input_count > 16
        || request.output_count > 16
        || request.flags > 1
        || request.method_result == 0
        || (request.input_count != 0 && request.inputs == 0)
        || (request.output_count != 0 && request.outputs == 0)
    {
        return Err("Synchronous-call descriptor exceeds the bounded probe contract".into());
    }
    Ok(request)
}
/// Operation 0x400 arguments recovered from the pinned library wrapper at 0x42f0.
/// Decoding addresses does not establish that their pointed-to memory is valid.
#[derive(Debug, Serialize)]
pub struct ConnectRequest {
    pub user_data: u64,
    pub user_data_bytes: u64,
    pub callback_status: u64,
    pub retry_flag: u64,
}
pub fn decode_connect_request(bytes: &[u8]) -> Result<ConnectRequest, String> {
    if bytes.len() != 32 {
        return Err(format!(
            "Expected 32 connection request bytes, captured {}",
            bytes.len()
        ));
    }
    let word = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    let request = ConnectRequest {
        user_data: word(0),
        user_data_bytes: word(8),
        callback_status: word(16),
        retry_flag: word(24),
    };
    if request.user_data_bytes != 0 && request.user_data == 0 {
        return Err("Nonempty connection payload has a null address".into());
    }
    if request.callback_status == 0 {
        return Err("Connection callback status has a null output address".into());
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synchronous_call_bounds_ignore_uninitialized_padding() {
        assert!(decode_sync_call(&[0; 47]).is_err());
        let mut bytes = [0u8; 48];
        bytes[32..40].copy_from_slice(&0x1000u64.to_le_bytes());
        bytes[12..16].fill(0xff);
        bytes[44..48].fill(0xff);
        assert_eq!(decode_sync_call(&bytes).unwrap().method_result, 0x1000);
        bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode_sync_call(&bytes).is_err());
        bytes[16..24].copy_from_slice(&0x2000u64.to_le_bytes());
        assert_eq!(decode_sync_call(&bytes).unwrap().input_count, 1);
        bytes[4..8].copy_from_slice(&17u32.to_le_bytes());
        assert!(decode_sync_call(&bytes).is_err());
        bytes[4..8].fill(0);
        bytes[40..44].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode_sync_call(&bytes).is_err());
    }
    #[test]
    fn connection_contract_rejects_truncation_and_inconsistent_pointers() {
        assert!(decode_connect_request(&[0; 31]).is_err());
        let mut bytes = [0u8; 32];
        assert!(decode_connect_request(&bytes).is_err());
        bytes[16..24].copy_from_slice(&0x1234u64.to_le_bytes());
        assert_eq!(
            decode_connect_request(&bytes).unwrap().callback_status,
            0x1234
        );
        bytes[8..16].copy_from_slice(&1u64.to_le_bytes());
        assert!(decode_connect_request(&bytes).is_err());
        bytes[..8].copy_from_slice(&0x5678u64.to_le_bytes());
        bytes[24..32].copy_from_slice(&0x90abu64.to_le_bytes());
        let request = decode_connect_request(&bytes).unwrap();
        assert_eq!(
            (
                request.user_data,
                request.user_data_bytes,
                request.retry_flag
            ),
            (0x5678, 1, 0x90ab)
        );
    }
}
