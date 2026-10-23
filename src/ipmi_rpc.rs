//! Bounded single-flight RPC channel for the synthetic native component test.
use std::{
    sync::{Condvar, Mutex},
    time::Duration,
};
#[derive(Clone, Debug)]
pub struct Request {
    pub client: u32,
    pub session: u32,
    pub caller_thread: u32,
    pub session_impl: u64,
    pub method: u32,
    pub input: Vec<u8>,
    pub generic_inputs: Option<Vec<Vec<u8>>>,
    pub capacity: usize,
}
impl Request {
    pub fn packet(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32 + self.input.len());
        bytes.extend(self.session_impl.to_le_bytes());
        bytes.extend(1u32.to_le_bytes()); // synthetic process identity
        bytes.extend(self.method.to_le_bytes());
        bytes.extend((self.generic_inputs.as_ref().map_or(1, Vec::len) as u32).to_le_bytes());
        bytes.extend(1u32.to_le_bytes());
        if let Some(inputs) = &self.generic_inputs {
            for input in inputs {
                bytes.extend((input.len() as u32).to_le_bytes());
                bytes.extend(input);
            }
        } else {
            bytes.extend((self.input.len() as u32).to_le_bytes());
            bytes.extend(&self.input);
        }
        bytes.extend((self.capacity as u32).to_le_bytes());
        bytes
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: i32,
    pub bytes: Vec<u8>,
}
#[derive(Default)]
struct State {
    request: Option<Request>,
    delivered_on: Option<u32>,
    response: Option<Response>,
    consumed: bool,
}
#[derive(Default)]
pub struct Rpc {
    state: Mutex<State>,
    changed: Condvar,
}
impl Rpc {
    pub fn enqueue(&self, request: Request) -> Result<(), &'static str> {
        let mut state = self.state.lock().unwrap();
        if state.request.is_some()
            || request.client == 0
            || request.session == 0
            || request.session_impl == 0
            || request.caller_thread == 0
            || request.input.len() > 4096
            || request.capacity > 4096
            || request.generic_inputs.as_ref().is_some_and(|inputs| {
                inputs.is_empty()
                    || inputs.len() > 10
                    || !request.input.is_empty()
                    || inputs.iter().any(|input| input.len() > 4096)
                    || inputs
                        .iter()
                        .try_fold(0usize, |sum, input| sum.checked_add(input.len()))
                        .is_none_or(|sum| sum > 4096)
            })
        {
            return Err("Invalid or duplicate synthetic RPC request");
        }
        state.request = Some(request);
        self.changed.notify_all();
        Ok(())
    }
    pub fn wait_ready(&self, timeout: Duration) -> Result<(), &'static str> {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.state.lock().unwrap(), timeout, |s| s.request.is_none())
            .unwrap();
        if state.request.is_some() {
            Ok(())
        } else {
            Err("Timed out waiting for RPC request")
        }
    }
    pub fn receive(&self, server_thread: u32) -> Option<Request> {
        let mut state = self.state.lock().unwrap();
        if state.delivered_on.is_some() || server_thread == 0 {
            return None;
        }
        let request = state.request.clone()?;
        if server_thread == request.caller_thread {
            return None;
        }
        state.delivered_on = Some(server_thread);
        Some(request)
    }
    pub fn respond(
        &self,
        session: u32,
        server_thread: u32,
        response: Response,
    ) -> Result<(), &'static str> {
        let mut state = self.state.lock().unwrap();
        let request = state
            .request
            .as_ref()
            .ok_or("Response without RPC request")?;
        if request.session != session
            || state.delivered_on != Some(server_thread)
            || state.response.is_some()
            || state.consumed
            || response.bytes.len() > request.capacity
        {
            return Err("Response does not match delivered RPC or capacity");
        }
        state.response = Some(response);
        self.changed.notify_all();
        Ok(())
    }
    pub fn wait_response(
        &self,
        client: u32,
        thread: u32,
        timeout: Duration,
    ) -> Result<Response, &'static str> {
        let state = self.state.lock().unwrap();
        if !state
            .request
            .as_ref()
            .is_some_and(|r| r.client == client && r.caller_thread == thread)
            || state.consumed
        {
            return Err("RPC wait identity mismatch or response already consumed");
        }
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |s| s.response.is_none())
            .unwrap();
        let response = state
            .response
            .take()
            .ok_or("Timed out waiting for native RPC response")?;
        state.consumed = true;
        Ok(response)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request {
            client: 2,
            session: 3,
            caller_thread: 10,
            session_impl: 0x8000,
            method: 7,
            input: vec![1, 2],
            generic_inputs: None,
            capacity: 4,
        }
    }
    #[test]
    fn generic_packet_layout_and_native_count_bounds() {
        let mut r = request();
        r.input.clear();
        r.generic_inputs = Some(vec![vec![1; 8], vec![2; 16], vec![3; 100]]);
        let bytes = r.packet();
        assert_eq!(bytes.len(), 164);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 8);
        assert_eq!(&bytes[28..36], &[1; 8]);
        assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 16);
        assert_eq!(&bytes[40..56], &[2; 16]);
        assert_eq!(u32::from_le_bytes(bytes[56..60].try_into().unwrap()), 100);
        assert_eq!(&bytes[60..160], &[3; 100]);
        assert_eq!(u32::from_le_bytes(bytes[160..164].try_into().unwrap()), 4);
        Rpc::default().enqueue(r.clone()).unwrap();
        for inputs in [
            vec![],
            vec![vec![]; 11],
            vec![vec![0; 4097]],
            vec![vec![0; 2049]; 2],
        ] {
            r.generic_inputs = Some(inputs);
            assert!(Rpc::default().enqueue(r.clone()).is_err());
        }
        r.generic_inputs = Some(vec![vec![]; 10]);
        Rpc::default().enqueue(r.clone()).unwrap();
        r.input.push(1);
        assert!(Rpc::default().enqueue(r).is_err());
    }
    #[test]
    fn packet_layout_and_response_identity_capacity() {
        let rpc = Rpc::default();
        assert!(rpc.wait_ready(Duration::ZERO).is_err());
        assert!(rpc
            .respond(
                3,
                20,
                Response {
                    status: 0,
                    bytes: vec![]
                }
            )
            .is_err());
        let packet = request().packet();
        assert_eq!(packet.len(), 34);
        assert_eq!(&packet[24..], &[2, 0, 0, 0, 1, 2, 4, 0, 0, 0]);
        rpc.enqueue(request()).unwrap();
        assert!(rpc.enqueue(request()).is_err());
        assert!(rpc.receive(10).is_none());
        assert!(rpc.wait_response(2, 10, Duration::ZERO).is_err());
        assert!(rpc.wait_response(2, 11, Duration::ZERO).is_err());
        assert_eq!(rpc.receive(20).unwrap().session, 3);
        assert!(rpc.receive(20).is_none());
        assert!(rpc
            .respond(
                3,
                21,
                Response {
                    status: 0,
                    bytes: vec![]
                }
            )
            .is_err());
        assert!(rpc
            .respond(
                4,
                20,
                Response {
                    status: 0,
                    bytes: vec![]
                }
            )
            .is_err());
        assert!(rpc
            .respond(
                3,
                20,
                Response {
                    status: 0,
                    bytes: vec![0; 5]
                }
            )
            .is_err());
        rpc.respond(
            3,
            20,
            Response {
                status: -1,
                bytes: vec![3],
            },
        )
        .unwrap();
        assert_eq!(
            rpc.wait_response(2, 10, Duration::ZERO).unwrap(),
            Response {
                status: -1,
                bytes: vec![3]
            }
        );
        assert!(rpc.wait_response(2, 10, Duration::ZERO).is_err());
        assert!(rpc
            .respond(
                3,
                20,
                Response {
                    status: 0,
                    bytes: vec![]
                }
            )
            .is_err());
    }
}
