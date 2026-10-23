//! Evidence decoder for the hash-pinned native IPMI client-create descriptor.
//! Field names remain structural until their service semantics are established.
use serde::Serialize;

pub const CREATE_DESCRIPTOR_BYTES: usize = 0x150;

#[derive(Debug, Serialize)]
pub struct CreateClientRequest {
    pub client_impl: u64,
    pub service_name: u64,
    pub config: u64,
}
pub fn decode_create_request(bytes: &[u8]) -> Result<CreateClientRequest, String> {
    if bytes.len() != 24 {
        return Err(format!(
            "Expected 24 client-create request bytes, captured {}",
            bytes.len()
        ));
    }
    let pointer = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    Ok(CreateClientRequest {
        client_impl: pointer(0),
        service_name: pointer(8),
        config: pointer(16),
    })
}

#[derive(Debug, Serialize)]
pub struct CreateDescriptor {
    pub declared_bytes: u64,
    pub field_08: u32,
    pub fields_10_18: [u64; 2],
    pub field_20: u32,
    pub fields_28_30: [u64; 2],
    pub field_38: u32,
    pub rounded_fields_40_138: Vec<u64>,
    pub field_140: u32,
    pub field_144: u32,
    pub field_148: u64,
    pub diagnostics: Vec<String>,
}
pub fn decode_create_descriptor(bytes: &[u8]) -> Result<CreateDescriptor, String> {
    if bytes.len() != CREATE_DESCRIPTOR_BYTES {
        return Err(format!(
            "Expected {CREATE_DESCRIPTOR_BYTES} descriptor bytes, captured {}",
            bytes.len()
        ));
    }
    let u64_at = |offset| u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
    let u32_at = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let declared_bytes = u64_at(0);
    let rounded_fields: Vec<_> = (0x40..0x140).step_by(8).map(u64_at).collect();
    let mut diagnostics = vec![];
    if declared_bytes != CREATE_DESCRIPTOR_BYTES as u64 {
        diagnostics.push("Size field does not match the observed native descriptor ABI".into());
    }
    if rounded_fields.iter().any(|value| value & 0x3fff != 0) {
        diagnostics.push("Rounded fields contain values not aligned to 16 KiB".into());
    }
    Ok(CreateDescriptor {
        declared_bytes,
        field_08: u32_at(8),
        fields_10_18: [u64_at(0x10), u64_at(0x18)],
        field_20: u32_at(0x20),
        fields_28_30: [u64_at(0x28), u64_at(0x30)],
        field_38: u32_at(0x38),
        rounded_fields_40_138: rounded_fields,
        field_140: u32_at(0x140),
        field_144: u32_at(0x144),
        field_148: u64_at(0x148),
        diagnostics,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_partial_capture_and_preserves_opaque_fields() {
        assert!(decode_create_request(&[0; 16]).is_err());
        let mut request = [0u8; 24];
        request[8..16].copy_from_slice(&0x8877665544332211u64.to_le_bytes());
        assert_eq!(
            decode_create_request(&request).unwrap().service_name,
            0x8877665544332211
        );
        assert!(decode_create_descriptor(&[0; 256]).is_err());
        let mut bytes = [0u8; CREATE_DESCRIPTOR_BYTES];
        bytes[..8].copy_from_slice(&(CREATE_DESCRIPTOR_BYTES as u64).to_le_bytes());
        bytes[0x40..0x48].copy_from_slice(&0x4000u64.to_le_bytes());
        bytes[0x148..0x150].copy_from_slice(&0x1122334455667788u64.to_le_bytes());
        let decoded = decode_create_descriptor(&bytes).unwrap();
        assert!(decoded.diagnostics.is_empty());
        assert_eq!(decoded.rounded_fields_40_138.len(), 32);
        assert_eq!(decoded.rounded_fields_40_138[0], 0x4000);
        assert_eq!(decoded.field_148, 0x1122334455667788);
        bytes[0x40] = 1;
        assert_eq!(
            decode_create_descriptor(&bytes).unwrap().diagnostics.len(),
            1
        );
    }
}
