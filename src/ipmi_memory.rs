//! Bounded copying of the measured client-create inputs, independent of host pointers.
//! The caller supplies memory access for its own guest/worker context. This does
//! not allocate handles, write guest outputs, authenticate processes or connect services.
use crate::ipmi::{self, CreateClientRequest, CreateDescriptor};

pub struct ClientInput {
    pub request: CreateClientRequest,
    pub descriptor: CreateDescriptor,
    pub service: String,
}

fn copy<R>(read: &mut R, address: u64, bytes: usize) -> Result<Vec<u8>, String>
where
    R: FnMut(u64, usize) -> Result<Vec<u8>, String>,
{
    if address == 0 || address.checked_add(bytes as u64).is_none() {
        return Err("Client input address is null or overflows".into());
    }
    let value = read(address, bytes)?;
    if value.len() != bytes {
        return Err("Memory provider returned a partial client input".into());
    }
    Ok(value)
}

pub fn client_create<R>(params: u64, bytes: usize, mut read: R) -> Result<ClientInput, String>
where
    R: FnMut(u64, usize) -> Result<Vec<u8>, String>,
{
    if bytes != 24 {
        return Err("Expected 24 client-create request bytes".into());
    }
    let request = ipmi::decode_create_request(&copy(&mut read, params, 24)?)?;
    let descriptor = ipmi::decode_create_descriptor(&copy(
        &mut read,
        request.config,
        ipmi::CREATE_DESCRIPTOR_BYTES,
    )?)?;
    if !descriptor.diagnostics.is_empty() {
        return Err("Descriptor does not match the recovered native ABI".into());
    }
    // Validate backing storage, while keeping the original implementation
    // address opaque. Only the original library interprets the object itself.
    copy(&mut read, request.client_impl, 16)?;
    let mut name = Vec::new();
    for offset in 0..25u64 {
        let address = request
            .service_name
            .checked_add(offset)
            .ok_or("Service name address overflow")?;
        let byte = copy(&mut read, address, 1)?[0];
        if byte == 0 {
            let service = String::from_utf8(name).map_err(|_| "Service name is not UTF-8")?;
            if service.is_empty() {
                return Err("Service name is empty".into());
            }
            return Ok(ClientInput {
                request,
                descriptor,
                service,
            });
        }
        name.push(byte);
    }
    Err("Service name exceeds the measured 24-byte bound".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u64 = 0x9000_0000_0000_0000;
    fn fixture(name: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 512];
        bytes[..8].copy_from_slice(&(BASE + 24).to_le_bytes());
        bytes[8..16].copy_from_slice(&(BASE + 400).to_le_bytes());
        bytes[16..24].copy_from_slice(&(BASE + 64).to_le_bytes());
        bytes[64..72].copy_from_slice(&(ipmi::CREATE_DESCRIPTOR_BYTES as u64).to_le_bytes());
        bytes[400..400 + name.len()].copy_from_slice(name);
        bytes
    }
    fn read(bytes: &[u8], address: u64, count: usize) -> Result<Vec<u8>, String> {
        let start = address
            .checked_sub(BASE)
            .ok_or("Outside synthetic memory")?;
        let start = usize::try_from(start).map_err(|_| "Offset too large")?;
        let end = start.checked_add(count).ok_or("Range overflow")?;
        bytes
            .get(start..end)
            .map(|v| v.to_vec())
            .ok_or("Outside synthetic memory".into())
    }

    #[test]
    fn opaque_addresses_and_terminal_nul_need_no_host_pointer() {
        let bytes = fixture(b"123456789012345678901234\0");
        let mut name_reads = 0;
        let input = client_create(BASE, 24, |a, n| {
            if a >= BASE + 400 {
                name_reads += 1;
                assert!(a <= BASE + 424);
            }
            read(&bytes, a, n)
        })
        .unwrap();
        assert_eq!(input.request.client_impl, BASE + 24);
        assert_eq!(input.service.len(), 24);
        assert_eq!(name_reads, 25);
    }

    #[test]
    fn request_size_and_partial_memory_fail_without_panic() {
        assert!(client_create(BASE, 23, |_, _| panic!("Must not read")).is_err());
        assert!(client_create(BASE, 24, |_, _| Ok(vec![0; 23])).is_err());
        assert!(client_create(u64::MAX - 8, 24, |_, _| panic!("Overflow must not read")).is_err());
    }

    #[test]
    fn descriptor_and_implementation_storage_must_be_readable() {
        let mut bytes = fixture(b"Test\0");
        bytes[64 + 0x40] = 1;
        assert!(client_create(BASE, 24, |a, n| read(&bytes, a, n)).is_err());
        bytes[64 + 0x40] = 0;
        assert!(client_create(BASE, 24, |a, n| {
            if a == BASE + 24 {
                return Err("Unreadable object".into());
            }
            read(&bytes, a, n)
        })
        .is_err());
    }

    #[test]
    fn names_reject_invalid_utf8_empty_and_missing_terminator() {
        for name in [b"\0".as_slice(), b"\xff\0", b"1234567890123456789012345"] {
            let bytes = fixture(name);
            assert!(client_create(BASE, 24, |a, n| read(&bytes, a, n)).is_err());
        }
        let bytes = fixture(b"X\0");
        let input = client_create(BASE, 24, |a, n| {
            assert_ne!(a, BASE + 402, "Must stop at first NUL");
            read(&bytes, a, n)
        })
        .unwrap();
        assert_eq!(input.service, "X");
    }
}
