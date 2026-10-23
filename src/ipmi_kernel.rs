//! Research client-handle state, independent of native pointer access or UI.
//! Experimental object registry; it does not implement service methods or scheduling.
use crate::ipmi::CreateDescriptor;
use crate::ipmi_server::ServerDescriptor;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct Client {
    pub owner: u32,
    pub implementation: u64,
    pub service: String,
    pub descriptor: CreateDescriptor,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidRequest,
    Capacity,
    UnknownHandle,
    WrongOwner,
    DuplicateName,
    ServerNotFound,
    Busy,
}
#[derive(Serialize)]
pub struct Session {
    pub owner: u32,
    pub implementation: u64,
    pub server: u32,
    pub client: u32,
    pub user_data: u64,
}
#[derive(Serialize)]
pub struct Server {
    pub owner: u32,
    pub implementation: u64,
    pub service: String,
    pub descriptor: ServerDescriptor,
}
pub struct Kernel {
    clients: BTreeMap<u32, Client>,
    next_handle: u32,
    capacity: usize,
    servers: BTreeMap<u32, Server>,
    sessions: BTreeMap<u32, Session>,
}
impl Kernel {
    pub fn new(capacity: usize) -> Self {
        Self {
            clients: BTreeMap::new(),
            next_handle: 1,
            capacity,
            servers: BTreeMap::new(),
            sessions: BTreeMap::new(),
        }
    }
    pub fn create(
        &mut self,
        owner: u32,
        implementation: u64,
        service: String,
        descriptor: CreateDescriptor,
    ) -> Result<u32, Error> {
        if owner == 0
            || implementation == 0
            || service.is_empty()
            || service.len() > 24
            || service.contains('\0')
            || !descriptor.diagnostics.is_empty()
            || descriptor.declared_bytes != crate::ipmi::CREATE_DESCRIPTOR_BYTES as u64
            || descriptor.rounded_fields_40_138.len() != 32
            || descriptor
                .rounded_fields_40_138
                .iter()
                .any(|v| v & 0x3fff != 0)
        {
            return Err(Error::InvalidRequest);
        }
        if self.object_count() >= self.capacity || self.next_handle > i32::MAX as u32 {
            return Err(Error::Capacity);
        }
        let id = self.next_handle;
        self.next_handle += 1;
        self.clients.insert(
            id,
            Client {
                owner,
                implementation,
                service,
                descriptor,
            },
        );
        Ok(id)
    }
    pub fn client(&self, owner: u32, handle: u32) -> Result<&Client, Error> {
        let client = self.clients.get(&handle).ok_or(Error::UnknownHandle)?;
        if client.owner != owner {
            return Err(Error::WrongOwner);
        }
        Ok(client)
    }
    pub fn destroy(&mut self, owner: u32, handle: u32) -> Result<(), Error> {
        self.client(owner, handle)?;
        if self.sessions.values().any(|s| s.client == handle) {
            return Err(Error::Busy);
        }
        self.clients.remove(&handle);
        Ok(())
    }
    pub fn is_empty(&self) -> bool {
        self.object_count() == 0
    }
    fn object_count(&self) -> usize {
        self.clients.len() + self.servers.len() + self.sessions.len()
    }
    pub fn create_session(
        &mut self,
        owner: u32,
        server: u32,
        client: u32,
        implementation: u64,
        user_data: u64,
    ) -> Result<u32, Error> {
        self.server(owner, server)?;
        if implementation == 0
            || self.resolve_server(
                self.clients.get(&client).ok_or(Error::UnknownHandle)?.owner,
                client,
            )? != server
        {
            return Err(Error::InvalidRequest);
        }
        if self.sessions.values().any(|s| s.client == client) {
            return Err(Error::Busy);
        }
        if self.object_count() >= self.capacity || self.next_handle > i32::MAX as u32 {
            return Err(Error::Capacity);
        }
        let handle = self.next_handle;
        self.next_handle += 1;
        self.sessions.insert(
            handle,
            Session {
                owner,
                implementation,
                server,
                client,
                user_data,
            },
        );
        Ok(handle)
    }
    pub fn session(&self, owner: u32, handle: u32) -> Result<&Session, Error> {
        let session = self.sessions.get(&handle).ok_or(Error::UnknownHandle)?;
        if session.owner != owner {
            return Err(Error::WrongOwner);
        }
        Ok(session)
    }
    pub fn client_session(&self, owner: u32, client: u32) -> Result<(u32, &Session), Error> {
        self.client(owner, client)?;
        self.sessions
            .iter()
            .find(|(_, session)| session.client == client)
            .map(|(id, session)| (*id, session))
            .ok_or(Error::UnknownHandle)
    }
    pub fn destroy_session(&mut self, owner: u32, handle: u32) -> Result<(), Error> {
        self.session(owner, handle)?;
        self.sessions.remove(&handle);
        Ok(())
    }
    pub fn create_server(
        &mut self,
        owner: u32,
        implementation: u64,
        service: String,
        descriptor: ServerDescriptor,
    ) -> Result<u32, Error> {
        if owner == 0
            || implementation == 0
            || service.is_empty()
            || service.len() > 24
            || service.contains('\0')
            || descriptor.event_handler == 0
            || descriptor.declared_bytes != 56
        {
            return Err(Error::InvalidRequest);
        }
        if self
            .servers
            .values()
            .any(|server| server.service == service)
        {
            return Err(Error::DuplicateName);
        }
        if self.object_count() >= self.capacity || self.next_handle > i32::MAX as u32 {
            return Err(Error::Capacity);
        }
        let id = self.next_handle;
        self.next_handle += 1;
        self.servers.insert(
            id,
            Server {
                owner,
                implementation,
                service,
                descriptor,
            },
        );
        Ok(id)
    }
    pub fn server(&self, owner: u32, handle: u32) -> Result<&Server, Error> {
        let server = self.servers.get(&handle).ok_or(Error::UnknownHandle)?;
        if server.owner != owner {
            return Err(Error::WrongOwner);
        }
        Ok(server)
    }
    pub fn destroy_server(&mut self, owner: u32, handle: u32) -> Result<(), Error> {
        self.server(owner, handle)?;
        if self.sessions.values().any(|s| s.server == handle) {
            return Err(Error::Busy);
        }
        self.servers.remove(&handle);
        Ok(())
    }
    pub fn resolve_server(&self, owner: u32, client: u32) -> Result<u32, Error> {
        let client = self.client(owner, client)?;
        self.servers
            .iter()
            .find(|(_, server)| server.service == client.service)
            .map(|(handle, _)| *handle)
            .ok_or(Error::ServerNotFound)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn session_capacity_ownership_and_endpoint_lifetimes() {
        let mut kernel = Kernel::new(3);
        let server = kernel
            .create_server(10, 0x1000, "Test".into(), server_config())
            .unwrap();
        let client = kernel.create(20, 0x2000, "Test".into(), config()).unwrap();
        assert_eq!(
            kernel.create_session(20, server, client, 0x3000, 0),
            Err(Error::WrongOwner)
        );
        let session = kernel
            .create_session(10, server, client, 0x3000, 0)
            .unwrap();
        assert_eq!(kernel.client_session(20, client).unwrap().0, session);
        assert!(matches!(
            kernel.client_session(10, client),
            Err(Error::WrongOwner)
        ));
        assert_eq!(kernel.session(10, session).unwrap().client, client);
        assert_eq!(
            kernel.create_session(10, server, client, 0x4000, 0),
            Err(Error::Busy)
        );
        assert_eq!(
            kernel.create(20, 0x4000, "Other".into(), config()),
            Err(Error::Capacity)
        );
        assert_eq!(kernel.destroy(20, client), Err(Error::Busy));
        assert_eq!(kernel.destroy_server(10, server), Err(Error::Busy));
        assert_eq!(kernel.destroy_session(20, session), Err(Error::WrongOwner));
        kernel.destroy_session(10, session).unwrap();
        assert!(matches!(
            kernel.session(10, session),
            Err(Error::UnknownHandle)
        ));
        kernel.destroy(20, client).unwrap();
        kernel.destroy_server(10, server).unwrap();
        assert!(kernel.is_empty());
    }
    fn server_config() -> ServerDescriptor {
        let mut bytes = [0u8; 56];
        bytes[..8].copy_from_slice(&56u64.to_le_bytes());
        bytes[48..56].copy_from_slice(&0x8000u64.to_le_bytes());
        crate::ipmi_server::decode_server_descriptor(&bytes).unwrap()
    }
    #[test]
    fn server_names_ownership_and_shared_handle_namespace() {
        let mut kernel = Kernel::new(2);
        let server = kernel
            .create_server(10, 0x1000, "TestEndpoint".into(), server_config())
            .unwrap();
        assert_eq!(
            kernel.create_server(20, 0x2000, "TestEndpoint".into(), server_config()),
            Err(Error::DuplicateName)
        );
        let client = kernel
            .create(20, 0x3000, "TestEndpoint".into(), config())
            .unwrap();
        assert_ne!(server, client);
        assert_eq!(kernel.resolve_server(20, client), Ok(server));
        assert_eq!(kernel.resolve_server(10, client), Err(Error::WrongOwner));
        assert_eq!(kernel.destroy_server(20, server), Err(Error::WrongOwner));
        kernel.destroy_server(10, server).unwrap();
        assert_eq!(
            kernel.resolve_server(20, client),
            Err(Error::ServerNotFound)
        );
        assert_eq!(kernel.destroy_server(10, server), Err(Error::UnknownHandle));
        kernel.destroy(20, client).unwrap();
        assert!(kernel.is_empty());
        let replacement = kernel
            .create_server(10, 0x4000, "TestEndpoint".into(), server_config())
            .unwrap();
        assert_ne!(server, replacement);
    }
    fn config() -> CreateDescriptor {
        let mut bytes = [0u8; crate::ipmi::CREATE_DESCRIPTOR_BYTES];
        bytes[..8].copy_from_slice(&(crate::ipmi::CREATE_DESCRIPTOR_BYTES as u64).to_le_bytes());
        crate::ipmi::decode_create_descriptor(&bytes).unwrap()
    }
    #[test]
    fn ownership_cleanup_and_stale_handles() {
        let mut kernel = Kernel::new(1);
        let first = kernel
            .create(10, 0x1234, "TestService".into(), config())
            .unwrap();
        assert_eq!(kernel.destroy(20, first), Err(Error::WrongOwner));
        assert_eq!(
            kernel.create(10, 0x5678, "Other".into(), config()),
            Err(Error::Capacity)
        );
        kernel.destroy(10, first).unwrap();
        let second = kernel.create(10, 0x5678, "Other".into(), config()).unwrap();
        assert_ne!(first, second);
        assert_eq!(kernel.destroy(10, first), Err(Error::UnknownHandle));
        kernel.destroy(10, second).unwrap();
        assert!(kernel.is_empty());
        assert_eq!(
            kernel.create(0, 0x1234, "Test".into(), config()),
            Err(Error::InvalidRequest)
        );
    }
}
